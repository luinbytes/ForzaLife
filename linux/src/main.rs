use forzalife::{overlay, telemetry};
use std::{
    env,
    net::UdpSocket,
    process::{Child, Command, ExitCode},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

fn main() -> ExitCode {
    match run() {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("forzalife: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<u8, Box<dyn std::error::Error>> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None | Some("overlay") => {
            let port = parse_port(args.next())?;
            overlay::run(port)?;
            Ok(0)
        }
        Some("probe") => probe(parse_port(args.next())?),
        Some("session") => {
            let command: Vec<String> = args.collect();
            session(&command)
        }
        Some("-h" | "--help" | "help") => {
            print_help();
            Ok(0)
        }
        Some(other) => Err(format!("unknown command {other:?}; use --help").into()),
    }
}

fn parse_port(value: Option<String>) -> Result<u16, Box<dyn std::error::Error>> {
    Ok(value
        .or_else(|| env::var("FORZALIFE_PORT").ok())
        .as_deref()
        .unwrap_or("8080")
        .parse()
        .map_err(|_| "port must be an integer from 1 to 65535")?)
}

fn probe(port: u16) -> Result<u8, Box<dyn std::error::Error>> {
    let socket = UdpSocket::bind(("127.0.0.1", port))?;
    let mut packet = [0_u8; telemetry::PACKET_SIZE + 1];
    let size = socket.recv(&mut packet)?;
    let data = telemetry::parse(&packet[..size]).map_err(|error| format!("{error:?}"))?;
    println!(
        "FH6 packet: {} bytes, car {}, {:.1} km/h, {:.0} rpm, {:.0}% fuel",
        size,
        data.car_ordinal,
        data.speed_mps * 3.6,
        data.current_engine_rpm,
        data.fuel * 100.0
    );
    Ok(0)
}

fn session(command: &[String]) -> Result<u8, Box<dyn std::error::Error>> {
    let (program, arguments) = command
        .split_first()
        .ok_or("session requires a game command")?;
    let executable = env::current_exe()?;
    let mut game = Command::new(program).args(arguments).spawn()?;
    let mut overlay = match Command::new(executable).arg("overlay").spawn() {
        Ok(overlay) => overlay,
        Err(error) => {
            terminate(&mut game);
            let _ = game.wait();
            return Err(error.into());
        }
    };
    let stopping = Arc::new(AtomicBool::new(false));
    ctrlc::set_handler({
        let stopping = Arc::clone(&stopping);
        move || stopping.store(true, Ordering::SeqCst)
    })?;

    let exit = loop {
        if let Some(status) = game.try_wait()? {
            break status.code().unwrap_or(1);
        }
        if stopping.load(Ordering::SeqCst) {
            terminate(&mut game);
            break game.wait()?.code().unwrap_or(1);
        }
        thread::sleep(Duration::from_millis(100));
    };
    terminate(&mut overlay);
    let _ = overlay.wait();
    Ok(u8::try_from(exit).unwrap_or(1))
}

fn terminate(child: &mut Child) {
    unsafe {
        libc::kill(child.id() as i32, libc::SIGTERM);
    }
}

fn print_help() {
    println!(
        "ForzaLife Linux\n\n\
         Usage:\n  \
         forzalife overlay [PORT]       Run the overlay (default UDP port: 8080)\n  \
         forzalife probe [PORT]         Print one validated FH6 packet\n  \
         forzalife session COMMAND...   Run overlay for the lifetime of COMMAND"
    );
}
