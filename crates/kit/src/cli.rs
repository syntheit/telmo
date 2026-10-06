//! Arguments every module accepts.

pub struct Args {
    /// Use fake data instead of the real system.
    pub mock: bool,
    /// Print one snapshot as JSON and exit, instead of opening the UI.
    pub status: bool,
}

pub fn args() -> Args {
    let mut args = Args {
        mock: false,
        status: false,
    };
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--mock" => args.mock = true,
            "status" => args.status = true,
            "-h" | "--help" => {
                let name = std::env::args().next().unwrap_or_default();
                println!("usage: {name} [--mock] [status]");
                std::process::exit(0);
            }
            other => {
                eprintln!("unknown argument: {other}");
                std::process::exit(2);
            }
        }
    }
    args
}
