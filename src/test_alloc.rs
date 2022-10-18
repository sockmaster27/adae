use std::{
    alloc::{GlobalAlloc, System},
    cell::RefCell,
    marker::PhantomData,
};

use backtrace::Backtrace;

#[cfg(not(any(feature = "panic_alloc", test)))]
#[global_allocator]
static GLOBAL: TestAlloc<PrintError> = TestAlloc {
    inner: System,
    error: PhantomData,
};
#[cfg(any(feature = "panic_alloc", test))]
#[global_allocator]
static GLOBAL: TestAlloc<PanicError> = TestAlloc {
    inner: System,
    error: PhantomData,
};

thread_local!(static ALLOWED: RefCell<bool> = RefCell::new(true));

macro_rules! no_heap {
    {$body:block} => {{
        let g = crate::test_alloc::NoHeapGuard::new();
        let r = $body;
        drop(g);
        r
    }}
}

pub fn is_allowed() -> bool {
    ALLOWED.with(|a| *a.borrow())
}
pub fn set_allowed(allowed: bool) {
    ALLOWED.with(|a| *a.borrow_mut() = allowed);
}

pub struct TestAlloc<E: ErrorHandler> {
    inner: System,
    error: PhantomData<E>,
}
unsafe impl<E> GlobalAlloc for TestAlloc<E>
where
    E: ErrorHandler,
{
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        ALLOWED.with(|a| {
            let allowed = *a.borrow();
            if !allowed {
                set_allowed(true);
                E::error("Attempted to allocate heap memory on disallowed thread");
                set_allowed(false);
            }

            self.inner.alloc(layout)
        })
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        ALLOWED.with(|a| {
            let allowed = *a.borrow();
            if !allowed {
                set_allowed(true);
                E::error("Attempted to deallocate heap memory on disallowed thread");
                set_allowed(false);
            }

            self.inner.dealloc(ptr, layout)
        })
    }
}
unsafe impl<E> Send for TestAlloc<E> where E: ErrorHandler {}
unsafe impl<E> Sync for TestAlloc<E> where E: ErrorHandler {}

pub struct NoHeapGuard(bool);
impl NoHeapGuard {
    pub fn new() -> Self {
        let before = is_allowed();
        set_allowed(false);
        Self(before)
    }
}
impl Drop for NoHeapGuard {
    fn drop(&mut self) {
        set_allowed(self.0);
    }
}

pub trait ErrorHandler {
    fn error(msg: &str);
}

pub struct PrintError;
impl ErrorHandler for PrintError {
    fn error(msg: &str) {
        eprintln!("{}\n{:?}", msg, Backtrace::new());
    }
}

pub struct PanicError;
impl ErrorHandler for PanicError {
    fn error(msg: &str) {
        panic!("{}", msg);
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    #[test]
    fn alloc_dealloc_allowed() {
        let b = Box::new(5);
        drop(b);
    }

    #[test]
    #[should_panic]
    fn alloc_dealloc_disallowed() {
        no_heap! {{
            let b = Box::new(5);
            drop(b);
        }}
    }

    #[test]
    #[should_panic]
    fn alloc_disallowed() {
        let b = no_heap! {{Box::new(5)}};
        drop(b);
    }

    #[test]
    #[should_panic]
    fn dealloc_disallowed() {
        let b = Box::new(5);
        no_heap! {{drop(b);}}
    }

    #[test]
    fn move_to_thread() {
        let mut b = Box::new(5);

        let t = thread::spawn(move || {
            no_heap! {{
                *b += 1;
                b
            }}
        });

        let b = t.join().unwrap();
        assert_eq!(*b, 6);
        drop(b);
    }
}
