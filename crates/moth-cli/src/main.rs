//! The command-line executable for the Moth compiler and project services.

mod cli;
mod command_status;
mod scaffold;

fn main() -> std::process::ExitCode {
    moth::install_compiler_bug_hook();
    cli::start_cli()
}
