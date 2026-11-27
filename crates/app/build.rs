fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    #[cfg(target_os = "windows")]
    {
        if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
            let mut res = winresource::WindowsResource::new();
            res.set_icon_with_id("assets/icon.ico", "1");
            res.set("ProductName", "Seu Navegador");
            res.set("FileDescription", "Seu Navegador");
            if let Err(err) = res.compile() {
                panic!("failed to compile Windows icon resource: {err}");
            }
        }
    }
}
