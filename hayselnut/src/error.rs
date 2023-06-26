use std::fmt::Debug;

#[derive(Debug, thiserror::Error)]
#[error("{{ no error was provided }}")]
pub struct EmptyError;

pub fn _panic_hwerr(error: impl Debug, message: &str) -> ! {
    panic!("\n\
        [E] -- Unexpected hardware error occured. --\n\
        [E] This is likely a code issue, or is otherwise caused bydamaged hardware / improper execution and may be fixed by restarting the chip.\n\
        [E] A description of what went wrong was provided:\n\
        [E]     {message}\n\
        [E] The error is as follows:\n\
        [E]     {error:#?}\n\
        [E] -- end error message --\n")
}

pub trait ErrExt {
    type T;
    type E;
    fn unwrap_hwerr(self, message: &str) -> Self::T;
}

impl<T, E: Debug> ErrExt for Result<T, E> {
    type T = T;
    type E = E;
    fn unwrap_hwerr(self, message: &str) -> Self::T {
        match self {
            Ok(v) => v,
            Err(error) => _panic_hwerr(error, message),
        }
    }
}
