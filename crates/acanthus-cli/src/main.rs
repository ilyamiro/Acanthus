fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(acanthus_cli::run(&args, &mut std::io::stdout(), &mut std::io::stderr()));
}
