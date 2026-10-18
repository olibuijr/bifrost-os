//! `leaks`: reports the memory a program leaks. The Linux tool for the job is valgrind's memcheck (in the runtime package
//! list), so `leaks --atExit -- PROGRAM [ARGS]` runs the program under `valgrind --leak-check=full` and reports what is
//! still allocated and unreachable when it exits; the exit status is 1 when a definite or indirect leak was found.
//! Gap: `leaks PID` and `leaks NAME`, which inspect a process that is already running, are refused with a message
//! (memcheck must start the program; there is no attach mode).

pub const USAGE: &str = "usage: leaks --atExit -- program [arguments]";
/// The program that does the work. It comes from the valgrind package.
pub const PROGRAM: &str = "valgrind";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Help,
    /// Run `PROGRAM` with these arguments.
    Run(Vec<String>),
}

/// Reads the arguments. `--atExit` (and the options that only change the report's layout, `--nocontext`,
/// `--nostacks`, `--quiet`) may come before `--`; the target program follows `--`, or follows the options directly.
pub fn parse(args: &[String]) -> Result<Plan, String> {
    let mut at_exit = false;
    let mut quiet = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "-help" | "--help" => return Ok(Plan::Help),
            "--atExit" => at_exit = true,
            "--quiet" => quiet = true,
            "--nocontext" | "--nostacks" => {}
            "--" => {
                i += 1;
                break;
            }
            a if a.starts_with('-') => return Err(format!("option {a} is not supported")),
            _ => break,
        }
        i += 1;
    }
    let target = &args[i.min(args.len())..];
    if target.is_empty() {
        return if args.is_empty() { Ok(Plan::Help) } else { Err("name the program to run after --atExit --".into()) };
    }
    if !at_exit && target.len() == 1 && target[0].chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("attaching to process {} is not supported: use leaks --atExit -- program", target[0]));
    }
    if !at_exit {
        return Err(format!("{} names a running process; only leaks --atExit -- program is supported", target[0]));
    }
    let mut out: Vec<String> = ["--leak-check=full", "--show-leak-kinds=definite,indirect", "--errors-for-leak-kinds=definite,indirect", "--error-exitcode=1"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    if quiet {
        out.push("--quiet".into());
    }
    out.extend(target.iter().cloned());
    Ok(Plan::Run(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn at_exit_runs_the_program_under_memcheck_and_fails_on_a_leak() {
        let Plan::Run(a) = parse(&args(&["--atExit", "--", "/bin/true", "-x"])).unwrap() else { panic!("run") };
        assert!(a.contains(&"--leak-check=full".to_string()));
        assert!(a.contains(&"--error-exitcode=1".to_string()));
        assert_eq!(&a[a.len() - 2..], ["/bin/true", "-x"]);
        let Plan::Run(b) = parse(&args(&["--atExit", "prog"])).unwrap() else { panic!("run") };
        assert_eq!(b.last().unwrap(), "prog");
        let Plan::Run(q) = parse(&args(&["--atExit", "--quiet", "--", "p"])).unwrap() else { panic!("run") };
        assert!(q.contains(&"--quiet".to_string()));
    }

    #[test]
    fn a_running_process_is_refused_and_help_is_the_default() {
        assert!(parse(&args(&["1234"])).unwrap_err().contains("attaching to process 1234 is not supported"));
        assert!(parse(&args(&["firefox"])).unwrap_err().contains("running process"));
        assert!(parse(&args(&["--atExit"])).unwrap_err().contains("name the program"));
        assert!(parse(&args(&["--bogus", "x"])).unwrap_err().contains("--bogus"));
        assert_eq!(parse(&[]), Ok(Plan::Help));
        assert_eq!(parse(&args(&["-h"])), Ok(Plan::Help));
    }
}
