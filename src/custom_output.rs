use std::sync::RwLock;

pub const ERR_MSG: &str = "A panic has previously ocurred while trying to set output function";

lazy_static! {
    pub static ref OUTPUTTER: RwLock<fn(String)> = RwLock::new(|_| {});
}

macro_rules! print {
    ($($arg:tt)*) => {{
        use $crate::custom_output::*;

        let msg = std::format!($($arg)*);
        let outputter = crate::custom_output::OUTPUTTER.read().expect(ERR_MSG);
        outputter(msg);
    }};
}

macro_rules! println {
    ($($arg:tt)*) => {{
        let msg = std::format!($($arg)*);
        print!("{}\n", msg);
    }};
}

macro_rules! eprint {
    ($($arg:tt)*) => {{
        let msg = std::format!($($arg)*);
        print!("ERROR: {}", msg);
    }};
}

macro_rules! eprintln {
    ($($arg:tt)*) => {{
        let msg = std::format!($($arg)*);
        eprint!("{}\n", msg);
    }};
}

// To be used in tests
#[allow(unused_macros)]
macro_rules! dbg {
    () => {
        eprintln!("[{}:{}]", std::file!(), std::line!())
    };
    ($val:expr $(,)?) => {
        match $val {
            tmp => {
                eprintln!("[{}:{}] {} = {:#?}",
                std::file!(), std::line!(), std::stringify!($val), &tmp);
                tmp
            }
        }
    };
    ($($val:expr),+ $(,)?) => {
        ($(dbg!($val)),+,)
    };
}

pub fn set_output(outputter: fn(String)) {
    let mut global_outputter = OUTPUTTER.write().expect(ERR_MSG);

    *global_outputter = outputter;
}
