#[allow(dead_code)]
#[path = "src/server_config.rs"]
mod server_config;
#[path = "src/update_repository.rs"]
mod update_repository;

fn main() {
    println!("cargo:rerun-if-env-changed=BENCHUP_UPDATE_REPOSITORY");
    println!("cargo:rerun-if-env-changed=GITHUB_REPOSITORY");
    println!("cargo:rerun-if-env-changed=KETCHUP_UPDATE_REPOSITORY");
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let repository = std::env::var("BENCHUP_UPDATE_REPOSITORY")
        .or_else(|_| std::env::var("KETCHUP_UPDATE_REPOSITORY"))
        .or_else(|_| std::env::var("GITHUB_REPOSITORY"))
        .ok()
        .or_else(|| {
            let output = std::process::Command::new("git")
                .args(["config", "--get", "remote.origin.url"])
                .current_dir(&root)
                .output()
                .ok()?;
            output
                .status
                .success()
                .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
        })
        .unwrap_or_else(|| update_repository::REPOSITORY.to_owned());
    let repository =
        update_repository::url(&repository).expect("Invalid desktop update repository");
    println!("cargo:rustc-env=BENCHUP_UPDATE_REPOSITORY_URL={repository}");
    println!("cargo:rerun-if-changed=src/update_repository.rs");
    // Re-evaluate local fork ownership if the origin is changed.
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--git-path", "config"])
        .current_dir(&root)
        .output()
    {
        if output.status.success() {
            let config = String::from_utf8_lossy(&output.stdout);
            println!(
                "cargo:rerun-if-changed={}",
                root.join(config.trim()).display()
            );
        }
    }
    println!("cargo:rerun-if-env-changed=BENCHUP_SERVER_URL");
    println!("cargo:rerun-if-env-changed=KETCHUP_SERVER_URL");
    println!("cargo:rerun-if-changed=src/server_config.rs");
    let dotenv = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.env");
    println!("cargo:rerun-if-changed={}", dotenv.display());
    let server = match std::env::var("BENCHUP_SERVER_URL").or_else(|error| match error {
        std::env::VarError::NotPresent => std::env::var("KETCHUP_SERVER_URL"),
        error => Err(error),
    }) {
        Ok(value) => Some(server_config::nonempty(&value).expect("Invalid BENCHUP_SERVER_URL")),
        Err(std::env::VarError::NotPresent) => {
            server_config::read_dotenv(&dotenv).expect("Invalid server .env")
        }
        Err(error) => panic!("Invalid BENCHUP_SERVER_URL: {error}"),
    };
    if let Some(server) = server {
        println!("cargo:rustc-env=BENCHUP_SERVER_URL={server}");
    }
}
