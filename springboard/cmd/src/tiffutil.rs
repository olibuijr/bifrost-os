//! `tiffutil`: looks at and rearranges TIFF files, through the libtiff tools in the runtime package list.
//!
//! * `-info FILE...` prints each file's directories (`tiffinfo`).
//! * `-cat FILE... -out OUT` joins the files' images into one multi-image file (`tiffcp`).
//! * `-extract N FILE -out OUT` writes image N (counting from 0) of FILE to OUT (`tiffcp FILE,N OUT`).
//! * `-lzw`, `-packbits`, `-none FILE -out OUT` recompress a file (`tiffcp -c lzw|packbits|none`).
//!
//! Refused with a message: `-cathidpicts`, `-catnosizecheck`, `-jpeg`, `-g4` and the other options of the reference tool
//! that have no `tiffcp` equivalent.

pub const USAGE: &str = "usage: tiffutil -info file... | -cat file... -out out | -extract n file -out out | -lzw|-packbits|-none file -out out";

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub enum Plan {
    Help,
    /// Run `program` with `args`.
    Run { program: &'static str, args: Vec<String> },
}

/// Splits the words after the operation into the input files and the `-out` target.
fn files_and_out(rest: &[&str]) -> Result<(Vec<String>, String), String> {
    let at = rest.iter().position(|w| *w == "-out").ok_or("-out file is required")?;
    let (inputs, out) = (&rest[..at], &rest[at + 1..]);
    match out {
        [one] if !one.starts_with('-') => Ok((inputs.iter().map(|s| s.to_string()).collect(), one.to_string())),
        _ => Err("-out takes exactly one file".into()),
    }
}

/// Reads the arguments.
pub fn parse(args: &[String]) -> Result<Plan, String> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let Some((op, rest)) = words.split_first() else { return Ok(Plan::Help) };
    match *op {
        "-h" | "-help" | "--help" => Ok(Plan::Help),
        "-info" => {
            if rest.is_empty() || rest.iter().any(|f| f.starts_with('-')) {
                return Err("-info needs one or more files".into());
            }
            Ok(Plan::Run { program: "tiffinfo", args: rest.iter().map(|s| s.to_string()).collect() })
        }
        "-cat" => {
            let (inputs, out) = files_and_out(rest)?;
            if inputs.is_empty() || inputs.iter().any(|f| f.starts_with('-')) {
                return Err("-cat needs one or more input files".into());
            }
            let mut a = inputs;
            a.push(out);
            Ok(Plan::Run { program: "tiffcp", args: a })
        }
        "-extract" => {
            let [n, file, tail @ ..] = rest else { return Err("-extract needs an image number, a file and -out".into()) };
            let n: u32 = n.parse().map_err(|_| format!("-extract needs an image number, not {n}"))?;
            let (extra, out) = files_and_out(tail)?;
            if !extra.is_empty() || file.starts_with('-') {
                return Err("-extract takes one input file".into());
            }
            Ok(Plan::Run { program: "tiffcp", args: vec![format!("{file},{n}"), out] })
        }
        "-lzw" | "-packbits" | "-none" => {
            let (inputs, out) = files_and_out(rest)?;
            if inputs.len() != 1 || inputs[0].starts_with('-') {
                return Err(format!("{op} takes one input file"));
            }
            let scheme = &op[1..];
            Ok(Plan::Run { program: "tiffcp", args: vec!["-c".into(), scheme.into(), inputs[0].clone(), out] })
        }
        other => Err(format!("{other} is not supported: tiffcp has no equivalent")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn info_goes_to_tiffinfo() {
        assert_eq!(parse(&args(&["-info", "a.tif", "b.tif"])), Ok(Plan::Run { program: "tiffinfo", args: args(&["a.tif", "b.tif"]) }));
        assert!(parse(&args(&["-info"])).is_err());
    }

    #[test]
    fn cat_joins_the_inputs_into_the_output() {
        assert_eq!(parse(&args(&["-cat", "a.tif", "b.tif", "-out", "all.tif"])), Ok(Plan::Run { program: "tiffcp", args: args(&["a.tif", "b.tif", "all.tif"]) }));
        assert!(parse(&args(&["-cat", "a.tif"])).unwrap_err().contains("-out"));
        assert!(parse(&args(&["-cat", "-out", "x.tif"])).unwrap_err().contains("input files"));
    }

    #[test]
    fn extract_picks_one_image_by_number() {
        assert_eq!(parse(&args(&["-extract", "1", "all.tif", "-out", "two.tif"])), Ok(Plan::Run { program: "tiffcp", args: args(&["all.tif,1", "two.tif"]) }));
        assert!(parse(&args(&["-extract", "x", "all.tif", "-out", "o.tif"])).unwrap_err().contains("image number"));
    }

    #[test]
    fn compression_choices_become_a_tiffcp_scheme() {
        assert_eq!(parse(&args(&["-lzw", "a.tif", "-out", "b.tif"])), Ok(Plan::Run { program: "tiffcp", args: args(&["-c", "lzw", "a.tif", "b.tif"]) }));
        assert_eq!(parse(&args(&["-none", "a.tif", "-out", "b.tif"])), Ok(Plan::Run { program: "tiffcp", args: args(&["-c", "none", "a.tif", "b.tif"]) }));
        assert!(parse(&args(&["-lzw", "a.tif", "b.tif", "-out", "c.tif"])).is_err());
    }

    #[test]
    fn other_options_are_refused_by_name() {
        assert!(parse(&args(&["-cathidpicts", "a", "b", "-out", "c"])).unwrap_err().contains("-cathidpicts is not supported"));
        assert_eq!(parse(&[]), Ok(Plan::Help));
    }
}
