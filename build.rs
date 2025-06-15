/* ---------------------------------------------------------------------------
** This software is in the public domain, furnished "as is", without technical
** support, and with no warranty, express or implied, as to its usefulness for
** any purpose.
**
** SPDX-License-Identifier: Unlicense
**
** -------------------------------------------------------------------------*/

use std::process::Command;

fn main() {
    let output = Command::new("git")
        .args(["describe", "--tags", "--always"])
        .output()
        .unwrap();

    let git_version = String::from_utf8_lossy(&output.stdout).trim().to_string();

    println!("cargo:rustc-env=GIT_VERSION={}", git_version);
}
