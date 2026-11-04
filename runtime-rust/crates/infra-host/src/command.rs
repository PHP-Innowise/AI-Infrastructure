//! Direct argv parsing. These helpers never invoke a command interpreter.
use crate::{HostError, Result};
/// Python shlex.split(..., posix=True, comments=False) compatible lexical rules:
/// ASCII shell whitespace, adjacent quoted fragments, retained empty quoted
/// arguments, and backslash removal only for quote/backslash in double quotes.
/// Expansion, operators and wildcard processing are caller responsibilities.
pub fn split_posix(command: &str) -> Result<Vec<String>> {
    if command.contains('\0') {
        return Err(HostError::invalid("Command contains NUL"));
    }
    #[derive(Clone, Copy)]
    enum Quote {
        None,
        Single,
        Double,
    }
    let mut quote = Quote::None;
    let mut input = command.chars();
    let mut words = Vec::new();
    let mut word = String::new();
    let mut present = false;
    while let Some(c) = input.next() {
        match quote {
            Quote::Single => {
                if c == '\'' {
                    quote = Quote::None;
                } else {
                    word.push(c);
                }
            }
            Quote::Double => match c {
                '"' => quote = Quote::None,
                '\\' => {
                    let c = input
                        .next()
                        .ok_or_else(|| HostError::invalid("Check command quoting."))?;
                    if !['"', '\\'].contains(&c) {
                        word.push('\\');
                    }
                    word.push(c);
                }
                _ => word.push(c),
            },
            Quote::None => match c {
                '\'' => {
                    quote = Quote::Single;
                    present = true;
                }
                '"' => {
                    quote = Quote::Double;
                    present = true;
                }
                '\\' => {
                    word.push(
                        input
                            .next()
                            .ok_or_else(|| HostError::invalid("Check command quoting."))?,
                    );
                    present = true;
                }
                ' ' | '\t' | '\r' | '\n' => {
                    if present {
                        words.push(std::mem::take(&mut word));
                        present = false;
                    }
                }
                _ => {
                    word.push(c);
                    present = true;
                }
            },
        }
    }
    if !matches!(quote, Quote::None) {
        return Err(HostError::invalid("Check command quoting."));
    }
    if present {
        words.push(word);
    }
    Ok(words)
}
#[cfg(not(windows))]
pub fn split_command(command: &str) -> Result<Vec<String>> {
    split_posix(command)
}
#[cfg(windows)]
pub fn split_command(command: &str) -> Result<Vec<String>> {
    if command.contains('\0') {
        return Err(HostError::invalid("Command contains NUL"));
    }
    #[link(name = "shell32")]
    unsafe extern "system" {
        fn CommandLineToArgvW(command: *const u16, count: *mut i32) -> *mut *mut u16;
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LocalFree(memory: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
    }
    struct Args(*mut *mut u16);
    impl Drop for Args {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0.cast());
            }
        }
    }
    let wide = format!("harness-check.exe {command}")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let mut count = 0;
    let args = unsafe { CommandLineToArgvW(wide.as_ptr(), &mut count) };
    if args.is_null() {
        return Err(HostError::invalid("Check command quoting."));
    }
    let args = Args(args);
    if count < 1 {
        return Err(HostError::invalid("Check command quoting."));
    }
    let mut result = Vec::new();
    for index in 1..count as usize {
        let raw = unsafe { *args.0.add(index) };
        if raw.is_null() {
            return Err(HostError::invalid("Check command quoting."));
        }
        let mut length = 0;
        unsafe {
            while *raw.add(length) != 0 {
                length += 1;
            }
        }
        let word = String::from_utf16(unsafe { std::slice::from_raw_parts(raw, length) })
            .map_err(|_| HostError::invalid("Check command is not valid Unicode."))?;
        result.push(word);
    }
    Ok(result)
}
