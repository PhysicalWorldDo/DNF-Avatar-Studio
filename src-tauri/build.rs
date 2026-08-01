fn main() {
    let dnflib_dir = std::path::PathBuf::from("third_party/dnflib");
    let src_dir = dnflib_dir.join("src");
    let include_dir = dnflib_dir.join("include");
    let source_files = [
        "miniz.c",
        "io_helper.c",
        "pixel.c",
        "dxt.c",
        "palette.c",
        "img.c",
        "npk.c",
        "dnflib.c",
    ];

    for source_file in &source_files {
        println!(
            "cargo:rerun-if-changed={}",
            src_dir.join(source_file).display()
        );
    }
    println!(
        "cargo:rerun-if-changed={}",
        include_dir.join("dnflib.h").display()
    );

    cc::Build::new()
        .include(include_dir)
        .include(&src_dir)
        .files(
            source_files
                .iter()
                .map(|source_file| src_dir.join(source_file)),
        )
        .warnings(false)
        .compile("dnflib");

    tauri_build::build();
}
