use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use git_rewrite::{CommitMeta, RepoManifest, collect_all_commits, export_tree};
use git2::Repository;

fn main() -> Result<()> {
	let repo_path = fs::canonicalize(Path::new("."))?;
	let repo = Repository::open(&repo_path)?;
	let branch = String::from("main");
	let commits = collect_all_commits(&repo, &branch)?;

	let repo_name = repo_path
		.file_name()
		.context("unable to get repo name")?
		.to_str()
		.context("unable to convert repo name to string")?
		.to_string();

	let mut manifest = RepoManifest {
		name: repo_name,
		branch,
		signing_keys: HashMap::new(),
		commits: Vec::new()
	};

	for (i, commit) in commits.iter().enumerate() {
		let folder = PathBuf::from(format!("export/{:04}_{}", i + 1, commit.id()));
		export_tree(&repo, commit.tree_id(), &folder)?;
		let meta = CommitMeta::from_commit(commit, &folder)?;
		fs::write(
			folder.join(".commit-meta.json"),
			serde_json::to_string_pretty(&meta)?
		)?;
		manifest.commits.push(meta);
	}

	fs::write(
		"export/manifest.json",
		serde_json::to_string_pretty(&manifest)?
	)?;

	Ok(())
}
