use std::sync::RwLock;

pub const ERR_MSG: &str = "A panic has previously ocurred while trying to set output function";

lazy_static! {
    pub static ref OUTPUTTER: RwLock<fn(String)> = RwLock::new(|_| {});
}

#[macro_export(crate)]
macro_rules! print {
    ($($arg:tt)*) => {{
        use $crate::custom_output::*;

        let msg = std::format!($($arg)*);
        let outputter = crate::custom_output::OUTPUTTER.read().expect(ERR_MSG);
        outputter(msg);
    }};
}

#[macro_export(crate)]
macro_rules! println {
    ($($arg:tt)*) => {{
        let msg = std::format!($($arg)*);
        print!("{}\n", msg);
    }};
}

#[macro_export(crate)]
macro_rules! eprint {
    ($($arg:tt)*) => {{
        let msg = std::format!($($arg)*);
        print!("ERROR: {}", msg);
    }};
}

#[macro_export(crate)]
macro_rules! eprintln {
    ($($arg:tt)*) => {{
        let msg = std::format!($($arg)*);
        eprint!("{}\n", msg);
    }};
}

pub fn set_output(outputter: fn(String)) {
    let mut global_outputter = OUTPUTTER.write().expect(ERR_MSG);

    *global_outputter = outputter;
}
