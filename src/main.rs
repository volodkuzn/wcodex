fn main() {
    if let Err(err) = wcodex::run::main_entry() {
        eprintln!("wcodex: {err:?}");
        std::process::exit(1);
    }
}
