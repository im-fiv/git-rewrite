use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use anyhow::Result;
use git_rewrite::{RepoManifest, replay_commit};
use git2::Repository;

fn main() -> Result<()> {
	// Load the manifest
	let manifest_str = fs::read_to_string("export/manifest.json")?;
	let manifest = serde_json::from_str::<RepoManifest>(&manifest_str)?;

	// Initialize new repository
	let repo_path = PathBuf::from(manifest.name);
	let repo = Repository::init(&repo_path)?;

	// Map old SHA -> new Oid
	let mut sha_map = HashMap::<String, git2::Oid>::new();

	// Replay commits in the exported order (topo order)
	for meta in &manifest.commits {
		let new_oid = replay_commit(&repo, meta, &sha_map)?;
		sha_map.insert(meta.sha.clone(), new_oid);
		println!("Replayed commit {} -> {}", &meta.sha[..8], new_oid);
	}

	// Create branch ref
	let head_commit = sha_map
		.get(&manifest.commits.last().unwrap().sha)
		.expect("Last commit missing");
	repo.branch(&manifest.branch, &repo.find_commit(*head_commit)?, true)?;

	println!("Reconstructed repository at {:?}", repo_path);
	Ok(())
}
