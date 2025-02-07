use colored::{Colorize};

/// Print an informational message in cyan.
pub fn print_info(msg: &str) {
    println!("{}", msg.cyan().bold());
}

/// Print a success message in green.
pub fn print_success(msg: &str) {
    println!("{}", msg.green().bold());
}

/// Print an error message in red to stderr.
pub fn print_error(msg: &str) {
    eprintln!("{}", msg.red().bold());
}
