use std::borrow::Cow;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::process::Command;

use rustyline::completion::{Completer, FilenameCompleter, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::{CmdKind, Highlighter};
use rustyline::hint::Hinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::Validator;
use rustyline::{Context, Editor, Helper};

const BUILTINS: [&str; 3] = ["cd", "ls", "exit"];

const GREEN: &str = "\x1b[1;32m";
const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[0m";

enum Flow {
    Continue,
    Exit,
}

fn main() {
    let mut editor: Editor<ShellHelper, DefaultHistory> = match Editor::new() {
        Ok(editor) => editor,
        Err(e) => {
            eprintln!("terminali avamine ebaõnnestus: {e}");
            return;
        }
    };
    editor.set_helper(Some(ShellHelper::new()));

    let history_path = env::var("HOME").ok().map(|home| format!("{home}/.rshell_history"));
    if let Some(path) = &history_path {
        let _ = editor.load_history(path);
    }

    loop {
        let prompt = format!("{}> ", current_dir_name());

        let line = match editor.readline(&prompt) {
            Ok(line) => line,
            Err(ReadlineError::Interrupted) => continue,
            Err(ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("viga sisendi lugemisel: {e}");
                break;
            }
        };

        if !line.trim().is_empty() {
            let _ = editor.add_history_entry(line.as_str());
        }

        if let Flow::Exit = execute(&line) {
            break;
        }
    }

    if let Some(path) = &history_path {
        if let Err(e) = editor.save_history(path) {
            eprintln!("ajaloo salvestamine ebaõnnestus: {e}");
        }
    }
}

fn current_dir_name() -> String {
    match env::current_dir() {
        Ok(dir) => dir.display().to_string(),
        Err(_) => String::from("?"),
    }
}

fn execute(line: &str) -> Flow {
    let words: Vec<String> = line.split_whitespace().flat_map(expand).collect();

    let (words, redirects) = match parse_redirects(words) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{e}");
            return Flow::Continue;
        }
    };

    let Some((cmd, args)) = words.split_first() else {
        return Flow::Continue;
    };

    if cmd == "exit" {
        return Flow::Exit;
    }

    let stdout_file = match create_file(redirects.stdout.as_deref()) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("{e}");
            return Flow::Continue;
        }
    };
    let stderr_file = match create_file(redirects.stderr.as_deref()) {
        Ok(file) => file,
        Err(e) => {
            eprintln!("{e}");
            return Flow::Continue;
        }
    };

    match cmd.as_str() {
        "cd" => {
            let mut err = output_or(stderr_file, io::stderr());
            report_write_error(cmd, cd(args, &mut err));
        }
        "ls" => {
            let mut out = output_or(stdout_file, io::stdout());
            let mut err = output_or(stderr_file, io::stderr());
            report_write_error(cmd, ls(args, &mut out, &mut err));
        }
        _ => run_external(cmd, args, stdout_file, stderr_file),
    }

    Flow::Continue
}

struct Redirects {
    stdout: Option<String>,
    stderr: Option<String>,
}

fn parse_redirects(words: Vec<String>) -> Result<(Vec<String>, Redirects), String> {
    let mut command = Vec::new();
    let mut redirects = Redirects {
        stdout: None,
        stderr: None,
    };
    let mut words = words.into_iter();

    while let Some(word) = words.next() {
        match word.as_str() {
            ">" => redirects.stdout = Some(words.next().ok_or("> järel puudub failinimi")?),
            "2>" => redirects.stderr = Some(words.next().ok_or("2> järel puudub failinimi")?),
            _ => command.push(word),
        }
    }

    Ok((command, redirects))
}

fn create_file(path: Option<&str>) -> Result<Option<File>, String> {
    match path {
        Some(path) => match File::create(path) {
            Ok(file) => Ok(Some(file)),
            Err(e) => Err(format!("{path}: {e}")),
        },
        None => Ok(None),
    }
}

fn output_or(file: Option<File>, default: impl Write + 'static) -> Box<dyn Write> {
    match file {
        Some(file) => Box::new(file),
        None => Box::new(default),
    }
}

fn report_write_error(cmd: &str, result: io::Result<()>) {
    if let Err(e) = result {
        eprintln!("{cmd}: väljundi kirjutamine ebaõnnestus: {e}");
    }
}

fn cd(args: &[String], err: &mut dyn Write) -> io::Result<()> {
    if args.len() > 1 {
        return writeln!(err, "cd: liiga palju argumente");
    }

    let target = match args.first() {
        Some(dir) => dir.clone(),
        None => match env::var("HOME") {
            Ok(home) => home,
            Err(_) => return writeln!(err, "cd: HOME pole määratud"),
        },
    };

    match env::set_current_dir(&target) {
        Ok(_) => (),
        Err(e) => {
            writeln!(err, "cd: {target}: {e}")?;
        }
    }

    Ok(())
}

fn ls(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> io::Result<()> {
    if args.is_empty() {
        return ls(&[String::from(".")], out, err);
    }

    for arg in args {
        let path = Path::new(arg);

        if path.is_dir() {
            match list_dir(path) {
                Ok(names) => {
                    for name in names {
                        writeln!(out, "{name}")?;
                    }
                }
                Err(e) => writeln!(err, "ls: {arg}: {e}")?,
            }
        } else if path.exists() {
            writeln!(out, "{arg}")?;
        } else {
            writeln!(err, "ls: {arg}: No such file or directory")?;
        }
    }

    Ok(())
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

fn run_external(cmd: &str, args: &[String], stdout: Option<File>, stderr: Option<File>) {
    let mut command = Command::new(cmd);
    command.args(args);

    match stdout {
        Some(file) => {
            command.stdout(file);
        }
        None => (),
    }

    match stderr {
        Some(file) => {
            command.stderr(file);
        }
        None => (),
    }

    match command.status() {
        Ok(status) if !status.success() => match status.code() {
            Some(code) => eprintln!("{cmd}: lõpetas koodiga {code}"),
            None => eprintln!("{cmd}: katkestati signaaliga"),
        },
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => eprintln!("{cmd}: käsku ei leitud"),
        Err(e) => eprintln!("{cmd}: {e}"),
    }
}

fn expand(word: &str) -> Vec<String> {
    let word = expand_tilde(word);
    let word = expand_dot(&word);

    if word.contains('*') {
        expand_star(&word)
    } else {
        vec![word]
    }
}

fn expand_tilde(word: &str) -> String {
    let home = match env::var("HOME") {
        Ok(home) => home,
        Err(_) => return word.to_string(),
    };

    if word == "~" {
        home
    } else if let Some(rest) = word.strip_prefix("~/") {
        format!("{home}/{rest}")
    } else {
        word.to_string()
    }
}

fn expand_dot(word: &str) -> String {
    let cwd = match env::current_dir() {
        Ok(cwd) => cwd,
        Err(_) => return word.to_string(),
    };

    let cwd = cwd.display();

    if word == "." {
        cwd.to_string()
    } else if let Some(rest) = word.strip_prefix("./") {
        format!("{cwd}/{rest}")
    } else {
        word.to_string()
    }
}

fn expand_star(word: &str) -> Vec<String> {
    let (dir, pattern) = match word.rfind('/') {
        Some(i) => (&word[..=i], &word[i + 1..]),
        None => ("", word),
    };
    let search_dir = if dir.is_empty() { "." } else { dir };

    let Ok(entries) = fs::read_dir(search_dir) else {
        return vec![word.to_string()];
    };

    let mut matches = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        let is_hidden = name.starts_with('.') && !pattern.starts_with('.');
        if !is_hidden && matches_pattern(pattern, &name) {
            matches.push(format!("{dir}{name}"));
        }
    }

    if matches.is_empty() {
        return vec![word.to_string()];
    }
    matches.sort();
    matches
}

fn matches_pattern(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();

    let [first, middle @ .., last] = parts.as_slice() else {
        return pattern == name;
    };

    let Some(mut rest) = name.strip_prefix(first) else {
        return false;
    };

    for part in middle {
        match rest.find(part) {
            Some(i) => rest = &rest[i + part.len()..],
            None => return false,
        }
    }

    rest.ends_with(last)
}

struct ShellHelper {
    files: FilenameCompleter,
}

impl ShellHelper {
    fn new() -> Self {
        ShellHelper {
            files: FilenameCompleter::new(),
        }
    }
}

fn command_names() -> Vec<String> {
    let mut names: Vec<String> = BUILTINS.iter().map(|name| name.to_string()).collect();

    if let Ok(path) = env::var("PATH") {
        for dir in path.split(':') {
            let Ok(entries) = fs::read_dir(dir) else {
                continue;
            };
            for entry in entries.flatten() {
                names.push(entry.file_name().to_string_lossy().to_string());
            }
        }
    }

    names.sort();
    names.dedup();
    names
}

impl Completer for ShellHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        ctx: &Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let before_cursor = line.get(..pos).unwrap_or(line);

        if before_cursor.trim_start().contains(' ') {
            return self.files.complete(line, pos, ctx);
        }

        let typed = before_cursor.trim_start();
        let start = before_cursor.len() - typed.len();

        let candidates = command_names()
            .into_iter()
            .filter(|name| name.starts_with(typed))
            .map(|name| Pair {
                display: name.clone(),
                replacement: name,
            })
            .collect();

        Ok((start, candidates))
    }
}

fn paint(text: &str, color: &str) -> String {
    format!("{color}{text}{RESET}")
}

impl Highlighter for ShellHelper {
    fn highlight<'l>(&self, line: &'l str, _pos: usize) -> Cow<'l, str> {
        let mut command_seen = false;

        let words: Vec<String> = line
            .split(' ')
            .map(|word| {
                if word.is_empty() {
                    String::new()
                } else if !command_seen {
                    command_seen = true;
                    paint(word, GREEN)
                } else if word == ">" || word == "2>" {
                    paint(word, YELLOW)
                } else {
                    word.to_string()
                }
            })
            .collect();

        Cow::Owned(words.join(" "))
    }

    fn highlight_char(&self, _line: &str, _pos: usize, _kind: CmdKind) -> bool {
        true
    }
}

impl Hinter for ShellHelper {
    type Hint = String;
}

impl Validator for ShellHelper {}

impl Helper for ShellHelper {}