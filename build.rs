fn main() {
    #[cfg(target_os = "windows")]
    {
        cc::Build::new()
            .cpp(true)
            .file("native/shell_menu.cpp")
            .define("UNICODE", None)
            .define("_UNICODE", None)
            .flag_if_supported("/std:c++17")
            .flag_if_supported("-std=c++17")
            .compile("ladon_shell_bridge");
        println!("cargo:rustc-link-lib=shell32");
        println!("cargo:rustc-link-lib=ole32");
        println!("cargo:rustc-link-lib=user32");
        println!("cargo:rustc-link-lib=uuid");
    }
}
