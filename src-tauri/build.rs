fn main() {
    // The build number reaches the binary through `option_env!`, and Cargo does
    // not track that by itself: with a warm build cache a new release would
    // keep the number it was first compiled with — exactly the case the number
    // exists to catch.
    println!("cargo:rerun-if-env-changed=REELDRIVE_BUILD");
    tauri_build::build();
}
