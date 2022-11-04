use std::{
    alloc::{GlobalAlloc, System},
    cell::RefCell,
    marker::PhantomData,
    thread,
};

use backtrace::Backtrace;

#[cfg(not(test))]
#[global_allocator]
static GLOBAL: TestAlloc<PrintError> = TestAlloc {
    inner: System,
    error: PhantomData,
};
#[cfg(test)]
#[global_allocator]
static GLOBAL: TestAlloc<PanicError> = TestAlloc {
    inner: System,
    error: PhantomData,
};

thread_local!(static ALLOWED: RefCell<bool> = RefCell::new(true));

macro_rules! no_heap {
    {$body:block} => {{
        let _g = crate::test_alloc::HeapGuard::new(false);
        let _r = $body;
        drop(_g);
        _r
    }}
}

macro_rules! allow_heap {
    {$body:block} => {{
        let _g = crate::test_alloc::HeapGuard::new(true);
        let _r = $body;
        drop(_g);
        _r
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

pub struct HeapGuard(bool);
impl HeapGuard {
    pub fn new(allowed: bool) -> Self {
        let before = is_allowed();
        set_allowed(allowed);
        Self(before)
    }
}
impl Drop for HeapGuard {
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
        if !thread::panicking() {
            panic!("{}", msg);
        }
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

    #[test]
    fn allow_inside() {
        let mut b = Box::new(5);
        no_heap! {{
            *b += 1;
            allow_heap! {{
                b = Box::new(2);
            }}
            *b += 1;
        }}
        assert_eq!(*b, 3)
    }

    #[test]
    #[should_panic]
    fn allow_disallow() {
        let mut b = Box::new(5);
        no_heap! {{
            allow_heap! {{
                *b += 1;
            }}
            drop(b);
        }}
    }

    #[test]
    #[should_panic]
    fn no_double_panic() {
        #[allow(unreachable_code)]
        {
            no_heap! {{
                panic!("123")
            }}
        }
    }
}
