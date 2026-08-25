# virglrenderer Venus fence + macOS WSI patches

Applied on slp `virglrenderer-0.10.4e-krunkit`. Rebuild:

```
meson setup build-smolvm -Dvenus=true -Drender-server=false \
  -Dc_args="-I/opt/homebrew/include -I/opt/homebrew/opt/molten-vk/include" \
  -Dc_link_args="-L/opt/homebrew/opt/molten-vk/lib"
meson compile -C build-smolvm
```

Copy `build-smolvm/src/libvirglrenderer.1.dylib` to `smolvm/lib/` with
`@loader_path` ids for MoltenVK and epoxy.

## What this fixes

- DRM-native stub returned -1, so `ASYNC_FENCE_CB|DRM` failed
  `virgl_renderer_init` and dropped Venus.
- `venus_context_retire_fences` printed UNIMPLEMENTED on every poll.
- Fence callback treated virtio `fence_id` as timeline seqno.
- Mesa WSI acquire imported sync-fd `-1`; MoltenVK has none, so the
  context went fatal and `vkWaitForFences` slept forever (Warzone black
  screen).
- Guest `VN_PERF=no_fence_feedback` is required so `vn_WaitForFences`
  calls host `vkGetFenceStatus` instead of a local feedback slot.
  Host GetFenceStatus must stay non-blocking (ring-thread
  `vkWaitForFences` deadlocks). Do not lie SUCCESS on NOT_READY.
- `vkr_log` also appends `/tmp/vkr.log`. Trailing zero padding on
  ExecuteCommandStreams SHM is skipped so leftover zeros are not
  decoded as `vkCreateInstance (0)`.
- Guest CPU must see HOST_VISIBLE blit memory. libkrun unmaps the
  virtio-gpu shm BAR once and `hv_vm_map`s each blob `map_ptr`
  (Mesa Venus / KVM_SET_USER_MEMORY_REGION). Warzone then paints.
