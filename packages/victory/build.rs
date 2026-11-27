use std::env;
use std::fs;
use std::path::PathBuf;

/**
 * Emit the linker input that exposes the D3D12 Agility SDK symbols.
 *
 * @remarks
 * The `#[unsafe(no_mangle)]` statics in `main.rs` are kept out of the PE
 * export directory by the MinGW linker. A module-definition file passed to
 * the linker creates the two exports the Agility SDK loader looks for.
 * Targets other than Windows take a no-op path.
 */
fn main() {
    let is_windows = env::var("CARGO_CFG_TARGET_OS")
        .map(|target_os| target_os == "windows")
        .unwrap_or(false);
    if !is_windows {
        return;
    }

    let out_dir =
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR for build scripts"));
    let def_path = out_dir.join("d3d12_agility_exports.def");
    fs::write(
        &def_path,
        "EXPORTS\n    D3D12SDKVersion DATA\n    D3D12SDKPath DATA\n",
    )
    .expect("failed to write the D3D12 Agility SDK export definition");

    println!("cargo:rustc-link-arg={}", def_path.display());
}
