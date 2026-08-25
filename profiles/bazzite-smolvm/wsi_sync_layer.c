/* VK_LAYER_SMOLVM_wsi_sync
 *
 * MoltenVK has no SYNC_FD. Mesa Venus still imports fd=-1 in
 * vn_AcquireNextImage* so the image looks ready. That never
 * signals, so games hang on a black window.
 *
 * This layer calls ICD acquire with no semaphore/fence, then
 * signals the app's semaphore/fence with an empty QueueSubmit.
 */
#define VK_NO_PROTOTYPES
#include <vulkan/vulkan.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <pthread.h>

#define LAYER_NAME "VK_LAYER_SMOLVM_wsi_sync"

#ifndef VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO
#define VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO ((VkStructureType)47)
#endif
#ifndef VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO
#define VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO ((VkStructureType)48)
#endif

typedef enum VkLayerFunction_ {
   VK_LAYER_LINK_INFO = 0,
   VK_LOADER_DATA_CALLBACK = 1
} VkLayerFunction;

typedef struct VkLayerInstanceLink_ {
   struct VkLayerInstanceLink_ *pNext;
   PFN_vkGetInstanceProcAddr pfnNextGetInstanceProcAddr;
   PFN_vkGetDeviceProcAddr pfnNextGetDeviceProcAddr;
} VkLayerInstanceLink;

typedef struct VkLayerDeviceLink_ {
   struct VkLayerDeviceLink_ *pNext;
   PFN_vkGetInstanceProcAddr pfnNextGetInstanceProcAddr;
   PFN_vkGetDeviceProcAddr pfnNextGetDeviceProcAddr;
} VkLayerDeviceLink;

typedef struct VkLayerInstanceCreateInfo {
   VkStructureType sType;
   const void *pNext;
   VkLayerFunction function;
   union {
      VkLayerInstanceLink *pLayerInfo;
      void *pfnSetInstanceLoaderData;
   } u;
} VkLayerInstanceCreateInfo;

typedef struct VkLayerDeviceCreateInfo {
   VkStructureType sType;
   const void *pNext;
   VkLayerFunction function;
   union {
      VkLayerDeviceLink *pLayerInfo;
      void *pfnSetDeviceLoaderData;
   } u;
} VkLayerDeviceCreateInfo;

static FILE *g_log;
static pthread_mutex_t g_lock = PTHREAD_MUTEX_INITIALIZER;

static void log_msg(const char *fmt, ...) {
   pthread_mutex_lock(&g_lock);
   if (!g_log) {
      g_log = fopen("/tmp/wsi-sync.log", "a");
      if (g_log)
         setvbuf(g_log, NULL, _IOLBF, 0);
   }
   if (g_log) {
      va_list ap;
      va_start(ap, fmt);
      vfprintf(g_log, fmt, ap);
      va_end(ap);
   }
   pthread_mutex_unlock(&g_lock);
}

struct device_data {
   VkDevice device;
   PFN_vkGetDeviceProcAddr get_device_proc_addr;
   PFN_vkGetDeviceQueue get_device_queue;
   PFN_vkQueueSubmit queue_submit;
   PFN_vkQueueWaitIdle queue_wait_idle;
   PFN_vkDeviceWaitIdle device_wait_idle;
   PFN_vkWaitForFences wait_for_fences;
   PFN_vkGetFenceStatus get_fence_status;
   PFN_vkAcquireNextImageKHR acquire;
   PFN_vkAcquireNextImage2KHR acquire2;
   PFN_vkQueuePresentKHR present;
   PFN_vkDestroyDevice destroy_device;
   VkQueue queue;
   struct device_data *next;
};

struct instance_data {
   VkInstance instance;
   PFN_vkGetInstanceProcAddr get_instance_proc_addr;
   PFN_vkDestroyInstance destroy_instance;
   struct instance_data *next;
};

static struct instance_data *g_instances;
static struct device_data *g_devices;

static struct instance_data *find_instance(VkInstance inst) {
   for (struct instance_data *p = g_instances; p; p = p->next)
      if (p->instance == inst)
         return p;
   return NULL;
}

static struct device_data *find_device(VkDevice dev) {
   for (struct device_data *p = g_devices; p; p = p->next)
      if (p->device == dev)
         return p;
   return NULL;
}

static void signal_acquire(struct device_data *d, VkSemaphore sem, VkFence fence) {
   if (!d || (sem == VK_NULL_HANDLE && fence == VK_NULL_HANDLE))
      return;
   if (d->queue == VK_NULL_HANDLE && d->get_device_queue)
      d->get_device_queue(d->device, 0, 0, &d->queue);
   if (d->queue == VK_NULL_HANDLE || !d->queue_submit) {
      log_msg("signal_acquire: no queue\n");
      return;
   }
   VkSubmitInfo info = {
      .sType = VK_STRUCTURE_TYPE_SUBMIT_INFO,
      .signalSemaphoreCount = sem != VK_NULL_HANDLE ? 1u : 0u,
      .pSignalSemaphores = sem != VK_NULL_HANDLE ? &sem : NULL,
   };
   VkResult r = d->queue_submit(d->queue, 1, &info, fence);
   if (r != VK_SUCCESS)
      log_msg("QueueSubmit signal result=%d\n", (int)r);
}

static VkResult VKAPI_CALL wrapped_AcquireNextImageKHR(
   VkDevice device, VkSwapchainKHR swapchain, uint64_t timeout,
   VkSemaphore semaphore, VkFence fence, uint32_t *pImageIndex) {
   struct device_data *d = find_device(device);
   if (!d || !d->acquire)
      return VK_ERROR_INITIALIZATION_FAILED;
   VkResult r = d->acquire(device, swapchain, timeout, VK_NULL_HANDLE,
                           VK_NULL_HANDLE, pImageIndex);
   if (r == VK_SUCCESS || r == VK_SUBOPTIMAL_KHR)
      signal_acquire(d, semaphore, fence);
   else
      log_msg("acquire failed %d\n", (int)r);
   return r;
}

static VkResult VKAPI_CALL wrapped_AcquireNextImage2KHR(
   VkDevice device, const VkAcquireNextImageInfoKHR *pInfo, uint32_t *pImageIndex) {
   struct device_data *d = find_device(device);
   if (!d || !d->acquire2)
      return VK_ERROR_INITIALIZATION_FAILED;
   VkAcquireNextImageInfoKHR info = *pInfo;
   VkSemaphore sem = info.semaphore;
   VkFence fence = info.fence;
   info.semaphore = VK_NULL_HANDLE;
   info.fence = VK_NULL_HANDLE;
   VkResult r = d->acquire2(device, &info, pImageIndex);
   if (r == VK_SUCCESS || r == VK_SUBOPTIMAL_KHR)
      signal_acquire(d, sem, fence);
   else
      log_msg("acquire2 failed %d\n", (int)r);
   return r;
}

static VkResult VKAPI_CALL wrapped_QueueWaitIdle(VkQueue queue) {
   /* Venus+MoltenVK never retires some binary fences (no SYNC_FD).
    * Warzone setupSwapchainImages blocks here forever after a layout
    * barrier submit. Returning success lets init finish; later waits
    * still go through WaitForFences with a bounded timeout.
    */
   log_msg("QueueWaitIdle skipped\n");
   (void)queue;
   return VK_SUCCESS;
}

static VkResult VKAPI_CALL wrapped_DeviceWaitIdle(VkDevice device) {
   log_msg("DeviceWaitIdle skipped\n");
   (void)device;
   return VK_SUCCESS;
}

static VkResult VKAPI_CALL wrapped_WaitForFences(
   VkDevice device, uint32_t fenceCount, const VkFence *pFences, VkBool32 waitAll,
   uint64_t timeout) {
   struct device_data *d = find_device(device);
   if (!d || !d->wait_for_fences)
      return VK_ERROR_INITIALIZATION_FAILED;
   uint64_t t = timeout;
   if (t > 50000000ull)
      t = 50000000ull;
   VkResult r = d->wait_for_fences(device, fenceCount, pFences, waitAll, t);
   if (r == VK_TIMEOUT) {
      log_msg("WaitForFences timeout -> SUCCESS (n=%u)\n", fenceCount);
      return VK_SUCCESS;
   }
   return r;
}

static VkResult VKAPI_CALL wrapped_GetFenceStatus(VkDevice device, VkFence fence) {
   struct device_data *d = find_device(device);
   if (!d || !d->get_fence_status)
      return VK_ERROR_INITIALIZATION_FAILED;
   VkResult r = d->get_fence_status(device, fence);
   if (r == VK_NOT_READY) {
      /* vn_WaitForFences polls this when allow_vk_wait_syncs=0. */
      return VK_SUCCESS;
   }
   return r;
}

static VkResult VKAPI_CALL wrapped_QueuePresentKHR(
   VkQueue queue, const VkPresentInfoKHR *pPresentInfo) {
   struct device_data *d = g_devices;
   if (!d || !d->present)
      return VK_ERROR_INITIALIZATION_FAILED;
   if (d->queue == VK_NULL_HANDLE)
      d->queue = queue;
   /* Warzone presentKHR waits on renderFinished. Venus+MoltenVK never
    * retires that semaphore (no SYNC_FD / fence feedback never writes
    * guest-visible memory), so the host queue stalls and
    * vn_queue_wsi_present's QueueWaitIdle never returns. Drop the waits
    * so the SW xcb/wl present path can copy the LINEAR image.
    */
   VkPresentInfoKHR info = *pPresentInfo;
   if (info.waitSemaphoreCount) {
      log_msg("QueuePresentKHR strip waits n=%u images=%u\n",
              info.waitSemaphoreCount, info.swapchainCount);
      info.waitSemaphoreCount = 0;
      info.pWaitSemaphores = NULL;
   } else {
      log_msg("QueuePresentKHR images=%u\n", info.swapchainCount);
   }
   VkResult r = d->present(queue, &info);
   if (r != VK_SUCCESS && r != VK_SUBOPTIMAL_KHR)
      log_msg("QueuePresentKHR result=%d\n", (int)r);
   return r;
}

static void VKAPI_CALL wrapped_DestroyDevice(VkDevice device,
                                             const VkAllocationCallbacks *alloc) {
   pthread_mutex_lock(&g_lock);
   struct device_data **pp = &g_devices;
   struct device_data *d = NULL;
   while (*pp) {
      if ((*pp)->device == device) {
         d = *pp;
         *pp = d->next;
         break;
      }
      pp = &(*pp)->next;
   }
   pthread_mutex_unlock(&g_lock);
   if (d) {
      if (d->destroy_device)
         d->destroy_device(device, alloc);
      free(d);
   }
}

static VkResult VKAPI_CALL wrapped_CreateDevice(
   VkPhysicalDevice physicalDevice, const VkDeviceCreateInfo *pCreateInfo,
   const VkAllocationCallbacks *pAllocator, VkDevice *pDevice) {
   const VkLayerDeviceCreateInfo *layer_info = NULL;
   for (const VkBaseInStructure *c = (const void *)pCreateInfo->pNext; c;
        c = c->pNext) {
      if (c->sType == VK_STRUCTURE_TYPE_LOADER_DEVICE_CREATE_INFO) {
         const VkLayerDeviceCreateInfo *li = (const void *)c;
         if (li->function == VK_LAYER_LINK_INFO) {
            layer_info = li;
            break;
         }
      }
   }
   if (!layer_info || !layer_info->u.pLayerInfo)
      return VK_ERROR_INITIALIZATION_FAILED;

   PFN_vkGetInstanceProcAddr gipa =
      layer_info->u.pLayerInfo->pfnNextGetInstanceProcAddr;
   PFN_vkGetDeviceProcAddr gdpa =
      layer_info->u.pLayerInfo->pfnNextGetDeviceProcAddr;
   ((VkLayerDeviceCreateInfo *)layer_info)->u.pLayerInfo =
      layer_info->u.pLayerInfo->pNext;

   PFN_vkCreateDevice next_create =
      (PFN_vkCreateDevice)gipa(VK_NULL_HANDLE, "vkCreateDevice");
   if (!next_create)
      return VK_ERROR_INITIALIZATION_FAILED;
   VkResult r = next_create(physicalDevice, pCreateInfo, pAllocator, pDevice);
   if (r != VK_SUCCESS)
      return r;

   struct device_data *d = calloc(1, sizeof(*d));
   d->device = *pDevice;
   d->get_device_proc_addr = gdpa;
   d->get_device_queue = (PFN_vkGetDeviceQueue)gdpa(*pDevice, "vkGetDeviceQueue");
   d->queue_submit = (PFN_vkQueueSubmit)gdpa(*pDevice, "vkQueueSubmit");
   d->queue_wait_idle = (PFN_vkQueueWaitIdle)gdpa(*pDevice, "vkQueueWaitIdle");
   d->device_wait_idle = (PFN_vkDeviceWaitIdle)gdpa(*pDevice, "vkDeviceWaitIdle");
   d->wait_for_fences = (PFN_vkWaitForFences)gdpa(*pDevice, "vkWaitForFences");
   d->get_fence_status = (PFN_vkGetFenceStatus)gdpa(*pDevice, "vkGetFenceStatus");
   d->acquire = (PFN_vkAcquireNextImageKHR)gdpa(*pDevice, "vkAcquireNextImageKHR");
   d->acquire2 = (PFN_vkAcquireNextImage2KHR)gdpa(*pDevice, "vkAcquireNextImage2KHR");
   d->present = (PFN_vkQueuePresentKHR)gdpa(*pDevice, "vkQueuePresentKHR");
   d->destroy_device = (PFN_vkDestroyDevice)gdpa(*pDevice, "vkDestroyDevice");
   if (d->get_device_queue)
      d->get_device_queue(*pDevice, 0, 0, &d->queue);
   pthread_mutex_lock(&g_lock);
   d->next = g_devices;
   g_devices = d;
   pthread_mutex_unlock(&g_lock);
   log_msg("CreateDevice ok queue=%p acquire=%p\n", (void *)d->queue,
           (void *)(uintptr_t)d->acquire);
   return VK_SUCCESS;
}

static PFN_vkVoidFunction VKAPI_CALL wrapped_GetDeviceProcAddr(VkDevice device,
                                                               const char *name);
static PFN_vkVoidFunction VKAPI_CALL wrapped_GetInstanceProcAddr(VkInstance instance,
                                                                 const char *name);

static VkResult VKAPI_CALL wrapped_CreateInstance(
   const VkInstanceCreateInfo *pCreateInfo, const VkAllocationCallbacks *pAllocator,
   VkInstance *pInstance) {
   const VkLayerInstanceCreateInfo *layer_info = NULL;
   for (const VkBaseInStructure *c = (const void *)pCreateInfo->pNext; c;
        c = c->pNext) {
      if (c->sType == VK_STRUCTURE_TYPE_LOADER_INSTANCE_CREATE_INFO) {
         const VkLayerInstanceCreateInfo *li = (const void *)c;
         if (li->function == VK_LAYER_LINK_INFO) {
            layer_info = li;
            break;
         }
      }
   }
   if (!layer_info || !layer_info->u.pLayerInfo)
      return VK_ERROR_INITIALIZATION_FAILED;
   PFN_vkGetInstanceProcAddr gipa =
      layer_info->u.pLayerInfo->pfnNextGetInstanceProcAddr;
   ((VkLayerInstanceCreateInfo *)layer_info)->u.pLayerInfo =
      layer_info->u.pLayerInfo->pNext;
   PFN_vkCreateInstance next_create =
      (PFN_vkCreateInstance)gipa(VK_NULL_HANDLE, "vkCreateInstance");
   if (!next_create)
      return VK_ERROR_INITIALIZATION_FAILED;
   VkResult r = next_create(pCreateInfo, pAllocator, pInstance);
   if (r != VK_SUCCESS)
      return r;
   struct instance_data *inst = calloc(1, sizeof(*inst));
   inst->instance = *pInstance;
   inst->get_instance_proc_addr = gipa;
   inst->destroy_instance =
      (PFN_vkDestroyInstance)gipa(*pInstance, "vkDestroyInstance");
   pthread_mutex_lock(&g_lock);
   inst->next = g_instances;
   g_instances = inst;
   pthread_mutex_unlock(&g_lock);
   log_msg("CreateInstance ok\n");
   return VK_SUCCESS;
}

static void VKAPI_CALL wrapped_DestroyInstance(VkInstance instance,
                                               const VkAllocationCallbacks *alloc) {
   pthread_mutex_lock(&g_lock);
   struct instance_data **pp = &g_instances;
   struct instance_data *inst = NULL;
   while (*pp) {
      if ((*pp)->instance == instance) {
         inst = *pp;
         *pp = inst->next;
         break;
      }
      pp = &(*pp)->next;
   }
   pthread_mutex_unlock(&g_lock);
   if (inst) {
      if (inst->destroy_instance)
         inst->destroy_instance(instance, alloc);
      free(inst);
   }
}

static PFN_vkVoidFunction VKAPI_CALL wrapped_GetDeviceProcAddr(VkDevice device,
                                                               const char *name) {
   if (!name)
      return NULL;
   if (strcmp(name, "vkAcquireNextImageKHR") == 0)
      return (PFN_vkVoidFunction)wrapped_AcquireNextImageKHR;
   if (strcmp(name, "vkAcquireNextImage2KHR") == 0)
      return (PFN_vkVoidFunction)wrapped_AcquireNextImage2KHR;
   if (strcmp(name, "vkQueuePresentKHR") == 0)
      return (PFN_vkVoidFunction)wrapped_QueuePresentKHR;
   if (strcmp(name, "vkQueueWaitIdle") == 0)
      return (PFN_vkVoidFunction)wrapped_QueueWaitIdle;
   if (strcmp(name, "vkDeviceWaitIdle") == 0)
      return (PFN_vkVoidFunction)wrapped_DeviceWaitIdle;
   if (strcmp(name, "vkWaitForFences") == 0)
      return (PFN_vkVoidFunction)wrapped_WaitForFences;
   if (strcmp(name, "vkGetFenceStatus") == 0)
      return (PFN_vkVoidFunction)wrapped_GetFenceStatus;
   if (strcmp(name, "vkDestroyDevice") == 0)
      return (PFN_vkVoidFunction)wrapped_DestroyDevice;
   if (strcmp(name, "vkGetDeviceProcAddr") == 0)
      return (PFN_vkVoidFunction)wrapped_GetDeviceProcAddr;
   struct device_data *d = find_device(device);
   if (d && d->get_device_proc_addr)
      return d->get_device_proc_addr(device, name);
   return NULL;
}

static PFN_vkVoidFunction VKAPI_CALL wrapped_GetInstanceProcAddr(VkInstance instance,
                                                                 const char *name) {
   if (!name)
      return NULL;
   if (strcmp(name, "vkCreateInstance") == 0)
      return (PFN_vkVoidFunction)wrapped_CreateInstance;
   if (strcmp(name, "vkDestroyInstance") == 0)
      return (PFN_vkVoidFunction)wrapped_DestroyInstance;
   if (strcmp(name, "vkGetInstanceProcAddr") == 0)
      return (PFN_vkVoidFunction)wrapped_GetInstanceProcAddr;
   if (strcmp(name, "vkCreateDevice") == 0)
      return (PFN_vkVoidFunction)wrapped_CreateDevice;
   if (strcmp(name, "vkGetDeviceProcAddr") == 0)
      return (PFN_vkVoidFunction)wrapped_GetDeviceProcAddr;
   if (strcmp(name, "vkAcquireNextImageKHR") == 0)
      return (PFN_vkVoidFunction)wrapped_AcquireNextImageKHR;
   if (strcmp(name, "vkAcquireNextImage2KHR") == 0)
      return (PFN_vkVoidFunction)wrapped_AcquireNextImage2KHR;
   if (strcmp(name, "vkQueuePresentKHR") == 0)
      return (PFN_vkVoidFunction)wrapped_QueuePresentKHR;
   if (strcmp(name, "vkQueueWaitIdle") == 0)
      return (PFN_vkVoidFunction)wrapped_QueueWaitIdle;
   if (strcmp(name, "vkDeviceWaitIdle") == 0)
      return (PFN_vkVoidFunction)wrapped_DeviceWaitIdle;
   if (strcmp(name, "vkWaitForFences") == 0)
      return (PFN_vkVoidFunction)wrapped_WaitForFences;
   if (strcmp(name, "vkGetFenceStatus") == 0)
      return (PFN_vkVoidFunction)wrapped_GetFenceStatus;
   struct instance_data *inst = find_instance(instance);
   if (inst && inst->get_instance_proc_addr)
      return inst->get_instance_proc_addr(instance, name);
   return NULL;
}

VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetInstanceProcAddr(VkInstance instance,
                                                               const char *pName) {
   return wrapped_GetInstanceProcAddr(instance, pName);
}
VKAPI_ATTR PFN_vkVoidFunction VKAPI_CALL vkGetDeviceProcAddr(VkDevice device,
                                                             const char *pName) {
   return wrapped_GetDeviceProcAddr(device, pName);
}

typedef enum VkNegotiateLayerStructType {
   LAYER_NEGOTIATE_INTERFACE_STRUCT = 1
} VkNegotiateLayerStructType;
typedef struct VkNegotiateLayerInterface {
   VkNegotiateLayerStructType sType;
   void *pNext;
   uint32_t loaderLayerInterfaceVersion;
   PFN_vkGetInstanceProcAddr pfnGetInstanceProcAddr;
   PFN_vkGetDeviceProcAddr pfnGetDeviceProcAddr;
   void *pfnGetPhysicalDeviceProcAddr;
} VkNegotiateLayerInterface;

VKAPI_ATTR VkResult VKAPI_CALL
vkNegotiateLoaderLayerInterfaceVersion(VkNegotiateLayerInterface *pVersionStruct) {
   if (!pVersionStruct)
      return VK_ERROR_INITIALIZATION_FAILED;
   if (pVersionStruct->loaderLayerInterfaceVersion < 2)
      return VK_ERROR_INITIALIZATION_FAILED;
   pVersionStruct->loaderLayerInterfaceVersion = 2;
   pVersionStruct->pfnGetInstanceProcAddr = vkGetInstanceProcAddr;
   pVersionStruct->pfnGetDeviceProcAddr = vkGetDeviceProcAddr;
   pVersionStruct->pfnGetPhysicalDeviceProcAddr = NULL;
   return VK_SUCCESS;
}
