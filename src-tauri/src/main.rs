fn main() {
    if let Some(exit_code) = codex_xray_lib::run_credential_helper_if_requested() {
        std::process::exit(exit_code);
    }
    codex_xray_lib::run();
}
