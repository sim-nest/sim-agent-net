use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::Command,
};

use sim_kernel::{Cx, Error, Result};

use super::spec::LineDriver;

pub(super) struct StdioLineDriver {
    multiline: bool,
    history_path: PathBuf,
}

impl StdioLineDriver {
    pub(super) fn new(multiline: bool) -> Self {
        Self {
            multiline,
            history_path: default_history_path(),
        }
    }
}

impl LineDriver for StdioLineDriver {
    fn read_line(&mut self, _cx: &mut Cx, prompt: &str) -> Result<Option<String>> {
        if self.multiline {
            read_multiline(prompt)
        } else {
            read_single_line(prompt)
        }
        .inspect(|line| {
            if let Some(line) = line
                && !line.trim().is_empty()
            {
                let _ = append_history(&self.history_path, line);
            }
        })
    }

    fn write_output(&mut self, _cx: &mut Cx, output: &str) -> Result<()> {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(output.as_bytes())
            .map_err(io_error_to_host)?;
        stdout.flush().map_err(io_error_to_host)
    }

    fn supports_multiline(&self) -> bool {
        self.multiline
    }
}

pub(super) struct ExternalDriver {
    cmd: String,
}

impl ExternalDriver {
    pub(super) fn new(cmd: String) -> Self {
        Self { cmd }
    }
}

impl LineDriver for ExternalDriver {
    fn read_line(&mut self, _cx: &mut Cx, prompt: &str) -> Result<Option<String>> {
        let mut path = env::temp_dir();
        path.push("sim-server-repl-input.lisp");
        write_editor_seed(&path, prompt)?;
        edit_path(&self.cmd, &path)?;
        let text = fs::read_to_string(&path).map_err(io_error_to_host)?;
        if text.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    fn write_output(&mut self, _cx: &mut Cx, output: &str) -> Result<()> {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(output.as_bytes())
            .map_err(io_error_to_host)?;
        stdout.flush().map_err(io_error_to_host)
    }
}

pub(super) struct BufferDriver {
    path: PathBuf,
}

impl BufferDriver {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl LineDriver for BufferDriver {
    fn read_line(&mut self, _cx: &mut Cx, prompt: &str) -> Result<Option<String>> {
        if !self.path.exists() {
            write_editor_seed(&self.path, prompt)?;
        }
        let cmd = env::var("EDITOR").unwrap_or_else(|_| "vi".to_owned());
        edit_path(&cmd, &self.path)?;
        let text = fs::read_to_string(&self.path).map_err(io_error_to_host)?;
        if text.trim().is_empty() {
            Ok(None)
        } else {
            Ok(Some(text))
        }
    }

    fn write_output(&mut self, _cx: &mut Cx, output: &str) -> Result<()> {
        let mut stdout = io::stdout().lock();
        stdout
            .write_all(output.as_bytes())
            .map_err(io_error_to_host)?;
        stdout.flush().map_err(io_error_to_host)
    }
}

fn read_single_line(prompt: &str) -> Result<Option<String>> {
    let mut stdout = io::stdout().lock();
    stdout
        .write_all(prompt.as_bytes())
        .map_err(io_error_to_host)?;
    stdout.flush().map_err(io_error_to_host)?;
    let mut line = String::new();
    let read = io::stdin()
        .lock()
        .read_line(&mut line)
        .map_err(io_error_to_host)?;
    if read == 0 {
        return Ok(None);
    }
    Ok(Some(line))
}

fn read_multiline(prompt: &str) -> Result<Option<String>> {
    let mut stdout = io::stdout().lock();
    let mut stdin = io::stdin().lock();
    let mut buf = String::new();
    let mut depth = 0isize;
    let mut current_prompt = prompt.to_owned();
    loop {
        stdout
            .write_all(current_prompt.as_bytes())
            .map_err(io_error_to_host)?;
        stdout.flush().map_err(io_error_to_host)?;
        let mut line = String::new();
        let read = stdin.read_line(&mut line).map_err(io_error_to_host)?;
        if read == 0 {
            return if buf.is_empty() {
                Ok(None)
            } else {
                Ok(Some(buf))
            };
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() && !buf.trim().is_empty() && depth <= 0 {
            break;
        }
        depth += paren_delta(&line);
        buf.push_str(&line);
        if depth <= 0 && !buf.trim().is_empty() && !trimmed.is_empty() {
            break;
        }
        current_prompt = ".. ".to_owned();
    }
    Ok(Some(buf))
}

fn paren_delta(line: &str) -> isize {
    let mut delta = 0isize;
    let mut in_string = false;
    let mut escaped = false;
    for ch in line.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        match ch {
            '\\' if in_string => escaped = true,
            '"' => in_string = !in_string,
            '(' | '[' | '{' if !in_string => delta += 1,
            ')' | ']' | '}' if !in_string => delta -= 1,
            _ => {}
        }
    }
    delta
}

fn default_history_path() -> PathBuf {
    let mut path = env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    path.push(".sim");
    path.push("history");
    path
}

fn append_history(path: &PathBuf, line: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(line.as_bytes())?;
    if !line.ends_with('\n') {
        file.write_all(b"\n")?;
    }
    Ok(())
}

fn write_editor_seed(path: &PathBuf, prompt: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(io_error_to_host)?;
    }
    fs::write(
        path,
        format!("\n; {prompt}Edit this buffer and save to submit.\n"),
    )
    .map_err(io_error_to_host)
}

fn edit_path(cmd: &str, path: &Path) -> Result<()> {
    let status = Command::new("sh")
        .arg("-lc")
        .arg(format!("{cmd} {}", shell_escape_path(path)))
        .status()
        .map_err(io_error_to_host)?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::HostError(format!(
            "editor command exited with status {status}"
        )))
    }
}

fn shell_escape_path(path: &Path) -> String {
    let raw = path.display().to_string();
    format!("'{}'", raw.replace('\'', "'\\''"))
}

fn io_error_to_host(err: io::Error) -> Error {
    Error::host_io(err)
}
