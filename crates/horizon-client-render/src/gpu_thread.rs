//! Where wgpu objects may go. Natively they are `Send` and `Sync`, and the renderer hands them
//! to helper threads (the submit thread, readbacks, screenshots, shader pack builds). In the
//! browser a WebGPU object belongs to the thread that made it, so those helpers run in place on
//! the render thread: [`spawn`] refuses there, and each caller already has a path for a thread
//! that couldn't start.

/// `Send` natively; anything in the browser, where nothing GPU-side leaves its thread.
#[cfg(not(target_arch = "wasm32"))]
pub trait MaybeSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send + ?Sized> MaybeSend for T {}
/// `Send` natively; anything in the browser, where nothing GPU-side leaves its thread.
#[cfg(target_arch = "wasm32")]
pub trait MaybeSend {}
#[cfg(target_arch = "wasm32")]
impl<T: ?Sized> MaybeSend for T {}

/// Work to run right after a frame's submission, on whichever thread submits.
#[cfg(not(target_arch = "wasm32"))]
pub type AfterSubmit = Box<dyn FnOnce() + Send>;
/// Work to run right after a frame's submission, on whichever thread submits.
#[cfg(target_arch = "wasm32")]
pub type AfterSubmit = Box<dyn FnOnce()>;

/// A named helper thread for work that holds GPU objects; in the browser an error, so the
/// caller does the work in place or goes without.
pub fn spawn<F, T>(name: &str, f: F) -> std::io::Result<std::thread::JoinHandle<T>>
where
    F: FnOnce() -> T + MaybeSend + 'static,
    T: MaybeSend + 'static,
{
    #[cfg(not(target_arch = "wasm32"))]
    return std::thread::Builder::new().name(name.into()).spawn(f);
    #[cfg(target_arch = "wasm32")]
    {
        let _ = (name, f);
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "WebGPU objects stay on the render thread",
        ))
    }
}

/// The error an error scope caught. Native scopes resolve at once. The browser's resolve later,
/// through its event loop, which a wait here would never reach: the scope is popped and its
/// error left to the device's uncaptured-error handler, which logs it.
pub fn pop_now(scope: wgpu::ErrorScopeGuard) -> Option<wgpu::Error> {
    #[cfg(not(target_arch = "wasm32"))]
    return pollster::block_on(scope.pop());
    #[cfg(target_arch = "wasm32")]
    {
        drop(scope.pop());
        None
    }
}
