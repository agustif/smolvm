# MoltenVK/Venus on SmolVM: fence feedback slots never become
# guest-visible, so vn_WaitForFences spins forever. Disable the
# slots so GetFenceStatus hits the host. Do not lie SUCCESS on
# NOT_READY (ResetFences on an unsignaled fence kills the ring).
export VN_PERF="${VN_PERF:-no_fence_feedback,no_semaphore_feedback,no_async_queue_submit}"
export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/virtio_icd.aarch64.json}"
export VK_LAYER_PATH="${VK_LAYER_PATH:-/usr/local/share/vulkan/explicit_layer.d}"
export VK_INSTANCE_LAYERS="${VK_INSTANCE_LAYERS:-VK_LAYER_SMOLVM_wsi_sync}"
export SDL_VIDEODRIVER="${SDL_VIDEODRIVER:-wayland}"
export SDL_VIDEO_DRIVER="${SDL_VIDEO_DRIVER:-wayland}"
# Venus sw WSI otherwise renders into LINEAR color attachments.
# MoltenVK/Metal cannot store those; the mapped swapchain stays black
# while the SDL/Wayland cursor (separate shm) still shows. Force a
# GPU->host buffer blit so present copies real pixels.
export MESA_VK_WSI_DEBUG="${MESA_VK_WSI_DEBUG:-buffer}"
