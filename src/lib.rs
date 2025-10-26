use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use chrono::{DateTime, FixedOffset};
use git2::{Commit, IndexAddOption, ObjectType, Oid, Repository, Signature};
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

pub mod time {
	use super::*;

	pub fn chrono_to_git2_time(date: &DateTime<FixedOffset>) -> git2::Time {
		// Seconds since epoch (UTC)
		let seconds = date.timestamp();
		// Offset in minutes
		let offset_minutes = date.offset().local_minus_utc() / 60;

		git2::Time::new(seconds, offset_minutes)
	}

	pub fn git2_to_chrono_date(time: &git2::Time) -> Result<DateTime<FixedOffset>> {
		let seconds = time.seconds();
		let offset =
			FixedOffset::east_opt(time.offset_minutes() * 60).context("invalid timezone offset")?;

		let date = DateTime::<FixedOffset>::from_naive_utc_and_offset(
			DateTime::from_timestamp(seconds, 0)
				.map(|d| d.naive_utc())
				.context("invalid commit timestamp")?,
			offset
		);

		Ok(date)
	}
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CommitMeta {
	pub sha: String,
	pub parents: Vec<String>,
	pub author_name: String,
	pub author_email: String,
	pub date: DateTime<FixedOffset>,
	pub message: String,
	pub tree_sha: String,
	pub folder: PathBuf
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RepoManifest {
	pub name: String,
	pub branch: String,
	pub commits: Vec<CommitMeta>
}

pub fn collect_all_commits<'repo>(
	repo: &'repo Repository,
	branch: &'_ str
) -> Result<Vec<Commit<'repo>>> {
	let mut revwalk = repo.revwalk()?;
	revwalk.push_ref(&format!("refs/heads/{}", branch))?;
	revwalk.set_sorting(git2::Sort::TOPOLOGICAL | git2::Sort::REVERSE)?;

	let mut commits = Vec::new();
	for oid in revwalk {
		let oid = oid?;
		let commit = repo.find_commit(oid)?;
		commits.push(commit);
	}

	Ok(commits)
}

pub fn export_tree(repo: &Repository, tree_oid: Oid, out_dir: &Path) -> Result<()> {
	let tree = repo.find_tree(tree_oid)?;

	for entry in tree.iter() {
		let name = entry.name().unwrap();
		let path = out_dir.join(name);

		match entry.kind() {
			Some(ObjectType::Blob) => {
				let parent = path.parent().context("file has no parent directory")?;
				fs::create_dir_all(parent)?;

				let blob = repo.find_blob(entry.id())?;
				fs::write(path, blob.content())?;
			}

			Some(ObjectType::Tree) => {
				fs::create_dir_all(&path)?;
				export_tree(repo, entry.id(), &path)?;
			}

			_ => {}
		}
	}

	Ok(())
}

pub fn copy_commit_files(src_dir: &Path, dest_dir: &Path) -> Result<()> {
	for entry in WalkDir::new(src_dir) {
		let entry = entry?;
		let path = entry.path();

		// Skip metadata
		if entry.file_name() == ".commit-meta.json" {
			continue;
		}

		let rel_path = path.strip_prefix(src_dir)?;
		let dest_path = dest_dir.join(rel_path);

		if entry.file_type().is_dir() {
			fs::create_dir_all(&dest_path)?;
		} else {
			if let Some(parent) = dest_path.parent() {
				fs::create_dir_all(parent)?;
			}

			fs::copy(path, &dest_path)?;
		}
	}

	Ok(())
}

pub fn replay_commit(
	repo: &Repository,
	meta: &CommitMeta,
	old_to_new: &HashMap<String, git2::Oid>
) -> Result<Oid> {
	let workdir = repo.workdir().context("repo has no working directory")?;

	// Clear current working directory
	for entry in fs::read_dir(workdir)? {
		let entry = entry?;
		let path = entry.path();

		if path.file_name() == Some(".git".as_ref()) {
			continue;
		}

		if path.is_dir() {
			fs::remove_dir_all(&path)?;
		} else {
			fs::remove_file(&path)?;
		}
	}

	// Copy files from exported commit folder
	copy_commit_files(&meta.folder, workdir)?;

	// Stage everything
	let mut index = repo.index()?;
	index.add_all(["*"], IndexAddOption::DEFAULT, None)?;
	index.write()?;

	let tree_oid = index.write_tree()?;
	let tree = repo.find_tree(tree_oid)?;

	// Author and committer
	let signature_time = time::chrono_to_git2_time(&meta.date);
	let author = Signature::new(&meta.author_name, &meta.author_email, &signature_time)?;
	let committer = author.clone();

	// Resolve new parent commits
	let parent_commits: Vec<Commit> = meta
		.parents
		.iter()
		.filter_map(|old_sha| old_to_new.get(old_sha))
		.filter_map(|&oid| repo.find_commit(oid).ok())
		.collect();

	let parent_refs: Vec<&Commit> = parent_commits.iter().collect();

	// Create commit
	let oid = repo.commit(
		Some("HEAD"),
		&author,
		&committer,
		&meta.message,
		&tree,
		&parent_refs
	)?;

	Ok(oid)
}

impl CommitMeta {
	pub fn from_commit(commit: &Commit, folder: impl AsRef<Path>) -> Result<Self> {
		// Author info
		let author = commit.author();
		let author_name = author.name().unwrap_or("unknown").to_string();
		let author_email = author.email().unwrap_or("unknown").to_string();

		// Commit message
		let message = commit.message().unwrap_or("").trim_end().to_string();

		// Convert date
		let time = commit.time();
		let date = time::git2_to_chrono_date(&time)?;

		// Collect parent SHAs
		let parents = commit
			.parent_ids()
			.map(|oid| oid.to_string())
			.collect::<Vec<_>>();

		Ok(Self {
			sha: commit.id().to_string(),
			parents,
			author_name,
			author_email,
			date,
			message,
			tree_sha: commit.tree_id().to_string(),
			folder: folder.as_ref().to_path_buf()
		})
	}
}
