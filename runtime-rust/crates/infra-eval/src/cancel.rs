use std::sync::atomic::{AtomicI32, Ordering};
static SIGNAL: AtomicI32 = AtomicI32::new(0);
pub fn signal() -> i32 {
    SIGNAL.load(Ordering::Acquire)
}
pub fn interrupted() -> bool {
    signal() != 0
}
#[cfg(unix)]
extern "C" fn handler(signal: i32) {
    SIGNAL.store(signal, Ordering::Release);
}
#[cfg(windows)]
#[link(name = "kernel32")]
unsafe extern "system" {
    fn SetConsoleCtrlHandler(
        handler: Option<unsafe extern "system" fn(u32) -> i32>,
        add: i32,
    ) -> i32;
}
#[cfg(windows)]
unsafe extern "system" fn console_handler(event: u32) -> i32 {
    SIGNAL.store(
        if event == 0 || event == 1 { 2 } else { 15 },
        Ordering::Release,
    );
    1
}
pub struct Scope {
    #[cfg(unix)]
    previous: Vec<(i32, libc::sigaction)>,
    #[cfg(windows)]
    console: bool,
}
impl Scope {
    pub fn install() -> crate::Result<Self> {
        SIGNAL.store(0, Ordering::Release);
        #[cfg(unix)]
        {
            let mut previous = vec![];
            for signal in [libc::SIGINT, libc::SIGTERM] {
                let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
                action.sa_sigaction = handler as *const () as usize;
                unsafe { libc::sigemptyset(&mut action.sa_mask) };
                action.sa_flags = 0;
                let mut old: libc::sigaction = unsafe { std::mem::zeroed() };
                if unsafe { libc::sigaction(signal, &action, &mut old) } != 0 {
                    for (sig, old) in &previous {
                        unsafe {
                            libc::sigaction(*sig, old, std::ptr::null_mut());
                        }
                    }
                    return Err(std::io::Error::last_os_error().into());
                }
                previous.push((signal, old));
            }
            Ok(Self { previous })
        }
        #[cfg(windows)]
        {
            if unsafe { SetConsoleCtrlHandler(Some(console_handler), 1) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Self { console: true })
        }
        #[cfg(not(any(unix, windows)))]
        {
            Ok(Self {})
        }
    }
}
impl Drop for Scope {
    fn drop(&mut self) {
        #[cfg(windows)]
        if self.console {
            unsafe {
                SetConsoleCtrlHandler(Some(console_handler), 0);
            }
        }
        #[cfg(unix)]
        for (signal, previous) in &self.previous {
            unsafe {
                libc::sigaction(*signal, previous, std::ptr::null_mut());
            }
        }
    }
}
