use std::process::Command;
use std::path::Path;

fn main() {
    // Ensure Python 3.13 is available
    let python = "python3.13";
    let status = Command::new(python)
        .arg("--version")
        .status()
        .expect("Failed to check python3.13 version");
    if !status.success() {
        panic!("python3.13 not found. Please install Python 3.13 and ensure it is on your PATH.");
    }

    // Create venv if not exists
    let venv_path = Path::new("pyenv");
    if !venv_path.exists() {
        let status = Command::new(python)
            .args(["-m", "venv", "pyenv"])
            .status()
            .expect("Failed to create venv");
        if !status.success() {
            panic!("Failed to create Python venv");
        }
    }

    // Install requirements.txt if present
    if Path::new("requirements.txt").exists() {
        let pip = if cfg!(target_os = "windows") {
            "pyenv\\Scripts\\pip"
        } else {
            "pyenv/bin/pip"
        };
        let status = Command::new(pip)
            .args(["install", "-r", "requirements.txt"])
            .status()
            .expect("Failed to install requirements.txt");
        if !status.success() {
            panic!("Failed to install Python requirements");
        }
    }
} 