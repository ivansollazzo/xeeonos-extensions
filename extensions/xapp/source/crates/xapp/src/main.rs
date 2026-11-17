//! xapp: the XeeonOS application tool (docs/xapp.md).
//!
//! A .xapp is one application in one file: put it in ~/Applications and it
//! is in the menu, delete it and it is gone, replace it and it is updated.
//! Every application runs in its own sandbox, with the permissions the user
//! gave it. This file only reads the command line and calls the crates.

mod commands;

use std::process::ExitCode;

const HELP: &str = "\
xapp: XeeonOS applications (.xapp)

  xapp run [--yes] [--trace] <file|app> [args...]
                                          start an application (also what a double click does);
                                          --trace: then tell what its sandbox refused
  xapp install [--yes] <file>             copy an application into ~/Applications
  xapp build <recipe> [--key <file>] [--output <file>] [--no-install]
                                          make a .xapp from a recipe, then install it
  xapp create --arch <package> | --aur <package> | --url <https://...> | --dir <directory>
              [--exec <program>] [--icon <image>] [--pkgbuild-ok]
              [--snapshot YYYY/MM/DD] [--id <app id>] [--output <dir>] [--no-build]
                                          wizard: make a recipe from an Arch package, an AUR
                                          package (its PKGBUILD shown first), an archive of
                                          the authors or a directory, then build it
  xapp list                               the applications in ~/Applications
  xapp info <app|file>                    what an application is, its author and permissions
  xapp verify <file>                      check signature and integrity, without opening
  xapp remove <app> [--purge]             delete its file (its data stays; --purge: data too)
  xapp permissions <app> [<permission> yes|no|ask]
                                          show or change what an application may do
  xapp doctor <app> [--reset]             offer what the last --trace run was refused;
                                          --reset: forget what was added that way
  xapp logs <app> [-f]                    the application's messages (journal)

--yes answers every question with its first choice (scripts).
Applications are not downloaded from a store: get the file, then run or install it.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match commands::dispatch(&args) {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("xapp: {e}");
            ExitCode::FAILURE
        }
    }
}
