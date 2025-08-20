use std::process::Command;

fn main() {
    Command::new("./node_modules/.bin/tailwindcss")
        .args([
            "-i",
            "./input.css",
            "-o",
            "./assets/tailwind.css",
            "--optimize",
        ])
        .status()
        .expect("Failed to run tailwindcss");
}
