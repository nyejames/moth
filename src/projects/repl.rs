//! Template-focused REPL helper for experimenting with Moth template syntax.
//!
//! This is not the default CLI entrypoint and it is narrower than a full language REPL: input is
//! tokenized from template-head mode and only compile-time template evaluation is supported today.

use saying::say;
use std::env;
use std::io::{self, Write};

/// Start the REPL session
#[allow(dead_code)] // Intentionally kept as pre-alpha placeholder for upcoming CLI REPL wiring.
pub fn start_repl_session() {
    say!(Red "REPL not yet implemented.");
    say!("Moth string template REPL");
    say!(Green "Enter Moth template snippets.");
    say!(Bright Black
        "Type 'exit' to quit. and 'clear' to restart the REPL or type 'show' to see the current code."
    );
    say!(Bright Black "This starts inside the template head. \n");

    // Just to avoid extra allocations, memory will not be much of a constraint in the repl (I think)
    const EXPECTED_INPUT_LENGTH: usize = 30;
    let mut code = String::with_capacity(EXPECTED_INPUT_LENGTH);

    loop {
        print!(">>> ");
        if let Err(error) = io::stdout().flush() {
            say!(Red "Error flushing prompt: ", error);
            break;
        }

        let _current_dir = match env::current_dir() {
            Ok(path) => path,
            Err(error) => {
                say!(Red "Error resolving current directory: ", error);
                break;
            }
        };

        let mut new_code = String::new();
        match io::stdin().read_line(&mut new_code) {
            Ok(_) => {
                if new_code.trim() == "exit" {
                    println!("Closing REPL session.");
                    break;
                }

                if new_code.trim() == "clear" {
                    code.clear();
                    continue;
                }

                if new_code.trim() == "show" {
                    println!("{code}");
                    continue;
                }

                let _next_code = format!("{code}{new_code}");
            }
            Err(e) => {
                say!(Red "Error reading input: ", e);
                break;
            }
        }
    }
}
