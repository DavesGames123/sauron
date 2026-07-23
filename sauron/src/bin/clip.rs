#[path = "../clip/mod.rs"]
mod clip;

fn main() {
    std::process::exit(clip::run_from_env());
}
