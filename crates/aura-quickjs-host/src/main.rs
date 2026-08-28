use aura_quickjs_host::{ProcessServer, QuickJsGuestEngine};
use std::io;

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if arguments.as_slice() != ["--stdio"] {
        eprintln!("usage: aura-quickjs-host --stdio");
        std::process::exit(2);
    }

    if let Err(error) =
        ProcessServer::new(io::stdin(), io::stdout(), QuickJsGuestEngine::default()).serve()
    {
        eprintln!("QuickJS Host protocol failure: {error}");
        std::process::exit(1);
    }
}
