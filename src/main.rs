#![allow(clippy::type_complexity)]

use std::fs::{self, DirEntry};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicU64, atomic::Ordering, Arc};
use std::thread;

use clap::Parser;
use indicatif::{style::TemplateError, ProgressBar, ProgressStyle};
use thiserror::Error;

#[derive(Debug, Clone, Parser)]
struct Command {
    sources: Vec<String>,
    destination: String,
    #[clap(short, long)]
    recursive: bool,
    #[clap(long)]
    noise: bool,
    #[clap(long = "no-progress")]
    no_progress_bar: bool,
}

#[allow(clippy::enum_variant_names)]
#[derive(Debug, Error)]
enum AntigError {
    #[error("{0}")]
    IOErr(#[from] std::io::Error),
    #[error("{0}")]
    IndicatifStyleErr(#[from] TemplateError),
    #[error("cannot copy a directory without recursive process")]
    CopyDirectoryWithoutRecursiveErr,
    #[error("{0} is not a directory")]
    IsNotDirErr(String),
}

fn main() -> Result<(), AntigError> {
    let mut command = Command::parse();

    if &command.destination == "." && command.sources.len() == 1 {
        command.destination = PathBuf::from(".")
            .canonicalize()?
            .join(PathBuf::from(&command.sources[0]).file_name().unwrap())
            .as_os_str()
            .to_string_lossy()
            .into_owned();
    }
    if !PathBuf::from(&command.destination).exists() {
        fs::create_dir(&command.destination)?;
    }

    let dir_content_size = Arc::new(AtomicU64::new(0));
    let bar = ProgressBar::new(100);

    for source in command.sources {
        let source_metadata = fs::metadata(&source)?;
        let destination_metadata = fs::metadata(&command.destination)?;

        if source_metadata.is_dir() {
            if !command.recursive {
                return Err(AntigError::CopyDirectoryWithoutRecursiveErr);
            }
            if !destination_metadata.is_dir() {
                return Err(AntigError::IsNotDirErr(command.destination.clone()));
            }

            get_files_count_recursive(
                &source,
                &command.destination,
                &dir_content_size,
                command.no_progress_bar,
            )?;

            copy_directory_recursive(
                &bar,
                &source,
                &command.destination,
                &dir_content_size,
                command.noise,
                command.no_progress_bar,
            )?;
        } else {
            let destination = if destination_metadata.is_dir() {
                PathBuf::from(&command.destination).join(&source)
            } else {
                PathBuf::from(&command.destination)
            };
            fs::copy(source, destination)?;
        }
    }

    Ok(())
}

fn visit_dir<const CREATE_DIR: bool>(
    dir: &Path,
    execlude: &Path,
    f: &mut dyn FnMut(&DirEntry) -> Result<(), AntigError>,
    g: Option<&dyn Fn(&DirEntry) -> Result<(), AntigError>>,
) -> Result<(), AntigError> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.canonicalize()? == execlude.canonicalize()? {
                continue;
            }

            if path.is_dir() {
                if CREATE_DIR {
                    g.unwrap()(&entry)?;
                }
                visit_dir::<CREATE_DIR>(&path, execlude, f, g)?;
            } else {
                f(&entry)?;
            }
        }
    }
    Ok(())
}

fn get_files_count_recursive(
    source: &str,
    destination: &str,
    dir_content_size: &Arc<AtomicU64>,
    no_progress_bar: bool,
) -> Result<(), AntigError> {
    if !no_progress_bar {
        let writer = Arc::clone(&dir_content_size);
        let source_clone = source.to_string();
        let destination_clone = destination.to_string();
        thread::spawn(move || {
            visit_dir::<false>(
                &PathBuf::from(source_clone),
                &PathBuf::from(destination_clone),
                &mut |_entry| -> Result<(), AntigError> {
                    writer.fetch_add(1, Ordering::Relaxed);
                    Ok(())
                },
                None,
            )
            .unwrap();
        });
    }

    Ok(())
}

fn copy_directory_recursive(
    bar: &ProgressBar,
    source: &str,
    destination: &str,
    dir_content_size: &Arc<AtomicU64>,
    noise: bool,
    no_progress_bar: bool,
) -> Result<(), AntigError> {
    bar.set_style(ProgressStyle::with_template(
        "{bar:60.cyan/blue} {pos:>7}/{len:7} {percent}% [{elapsed_precise}]",
    )?);

    visit_dir::<true>(
        &PathBuf::from(&source),
        &PathBuf::from(&destination),
        &mut |entry| -> Result<(), AntigError> {
            let destination =
                PathBuf::from(&destination).join(entry.path().strip_prefix(&source).unwrap());

            if noise {
                bar.println(format!(
                    "cp: {} => {}",
                    entry.path().display(),
                    destination.display(),
                ));
            }

            if !no_progress_bar {
                bar.set_length(dir_content_size.load(Ordering::Relaxed));
            }

            match fs::copy(entry.path(), &destination) {
                Ok(_) => {}
                Err(err) => match err.kind() {
                    std::io::ErrorKind::AlreadyExists => {
                        let entry_len = entry.metadata()?.len();
                        let destination_len = PathBuf::from(&destination).metadata()?.len();
                        if entry_len != destination_len {
                            fs::remove_file(&destination)?;
                            fs::copy(entry.path(), &destination)?;
                        }
                    }
                    _ => return Err(err.into()),
                },
            }

            if !no_progress_bar {
                bar.inc(1);
            }

            Ok(())
        },
        Some(&|entry| -> Result<(), AntigError> {
            let destination =
                PathBuf::from(&destination).join(entry.path().strip_prefix(&source).unwrap());
            match fs::create_dir(destination) {
                Ok(_) => {}
                Err(err) => match err.kind() {
                    std::io::ErrorKind::AlreadyExists => {}
                    _ => return Err(err.into()),
                },
            }
            Ok(())
        }),
    )?;

    Ok(())
}
