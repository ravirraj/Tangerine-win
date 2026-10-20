//! Compiles the Slint markup into Rust before the program is built.

fn main() {
    let path = "src/ui/wheel.slint";
    println!("cargo:rerun-if-changed={path}");
    slint_build::compile(path).expect("the wheel markup could not be compiled");
}
