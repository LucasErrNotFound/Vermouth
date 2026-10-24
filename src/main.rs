#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod app;
mod camera;
mod features;
mod persistence;

fn main() {
    app::run();
}
