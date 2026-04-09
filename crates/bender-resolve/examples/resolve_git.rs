use bender_resolve::fetcher::{DependencyFetcher, FetchConfig};
use bender_resolve::manifest::PartialManifest;
use bender_resolve::{BenderProvider, BenderVersion};

const ROOT_MANIFEST: &str = r#"
package:
  name: root

dependencies:
  cheshire:
    git: https://github.com/pulp-platform/cheshire.git
    version: 0.3.1
"#;

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(run());
}

async fn run() {
    let db_dir = std::env::temp_dir().join("bender-resolve-example");
    println!("Using db dir: {}", db_dir.display());

    let fetcher = DependencyFetcher::new(FetchConfig { db_dir });

    let mut provider = BenderProvider::new(fetcher);

    let manifest = PartialManifest::parse(ROOT_MANIFEST).expect("failed to parse manifest");
    let root_name = manifest
        .package
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "root".to_string());
    let root_version = BenderVersion::Semver(semver::Version::new(0, 0, 0));
    provider
        .init_root(&manifest, &root_name, root_version.clone())
        .expect("failed to init provider");

    println!("Resolving...");
    match bender_resolve::resolve(&provider, root_name.clone(), root_version) {
        Ok(solution) => {
            println!("\nResolved {} package(s):", solution.len() - 1);
            let mut packages: Vec<_> = solution
                .iter()
                .filter(|(name, _)| *name != &root_name)
                .collect();
            packages.sort_by_key(|(name, _)| name.as_str());
            for (name, version) in packages {
                println!("  {name} = {version}");
            }
        }
        Err(e) => {
            eprintln!("\nResolution failed:\n{e}");
        }
    }
}
