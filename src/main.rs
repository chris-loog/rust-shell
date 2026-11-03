use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;

fn main() {
    let mut line = String::new();

    loop {
        let cwd = env::current_dir().unwrap_or_default();
        let cwd_display_name = cwd.display();
        print!("{cwd_display_name}> ");
        io::stdout().flush().unwrap();

        line.clear();
        match io::stdin().read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("viga sisendi lugemisel: {e}");
                continue;
            }
        }

        let mut parts = line.split_whitespace();
        let Some(cmd) = parts.next() else { continue };
        let args: Vec<&str> = parts.collect();

        match cmd {
            "exit" => break,
            "cd" => cd(&args),
            "ls" => ls(&args),
            _ => eprintln!("{cmd}: käsku ei leitud"),
        }
    }
}

fn cd(args: &[&str]) {
    let target = match args.first() {
        Some(dir) => dir.to_string(),
        None => env::var("HOME").unwrap_or_default(),
    };
    if let Err(e) = env::set_current_dir(&target) {
        eprintln!("cd: {target}: {e}");
    }
}

fn ls(args: &[&str]) {
    let dir = args.first().copied().unwrap_or(".");
    match list_dir(Path::new(dir)) {
        Ok(names) => {
            for name in names {
                println!("{name}");
            }
        }
        Err(e) => eprintln!("ls: {dir}: {e}"),
    }
}

fn list_dir(dir: &Path) -> io::Result<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let mut name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('.') {
            continue;
        }
        if entry.file_type()?.is_dir() {
            name.push('/');
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}