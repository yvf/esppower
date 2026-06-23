fn main() {
    // linkall.x must be the last linker script (esp-hal).
    println!("cargo::rustc-link-arg=-Tlinkall.x");
}
