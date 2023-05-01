#![allow(clippy::type_complexity)]

use std::fs::{self, DirEntry};
use std::path::{Path, PathBuf};
use std::sync::{atomic::AtomicU64, atomic::Ordering, Arc};
use std::thread;

use clap::Parser;
use error_stack::{IntoReport, Report, Result, ResultExt};
use indicatif::{ProgressBar, ProgressStyle};
use thiserror::Error;

#[derive(Debug, Clone, Parser)]
struct Command {
    sources: Vec<String>,
    destination: String,
    #[clap(short, long)]
    /// Copy contents recursively. Usually it is used to copy a directory
    recursive: bool,
    #[clap(long)]
    /// To show coping infos
    noise: bool,
    #[clap(long = "no-progress")]
    /// Disable showing the progress bar
    no_progress_bar: bool,
}

#[derive(Debug, Error)]
#[error("Running antig failed")]
struct AntigErr;

fn main() -> Result<(), AntigErr> {
    let mut command = Command::parse();

    if &command.destination == "." && command.sources.len() == 1 {
        command.destination = PathBuf::from(".")
            .canonicalize()
            .into_report()
            .change_context(AntigErr)
            .attach_printable_lazy(|| "cannot get the canonicalize directory path.")?
            .join(PathBuf::from(&command.sources[0]).file_name().unwrap())
            .as_os_str()
            .to_string_lossy()
            .into_owned();
    }
    if !PathBuf::from(&command.destination).exists() {
        fs::create_dir(&command.destination)
            .into_report()
            .change_context(AntigErr)
            .attach_printable_lazy(|| {
                format!("cannot create a directory `{}`.", &command.destination)
            })?;
    }

    let dir_content_size = Arc::new(AtomicU64::new(0));
    let bar = ProgressBar::new(100);
    bar.set_style(
        ProgressStyle::with_template(
            "{bar:60.cyan/blue} {pos:>7}/{len:7} {percent}% [{elapsed_precise}]",
        )
        .into_report()
        .change_context(AntigErr)
        .attach_printable_lazy(|| "there is some error to change the progress bar style.")?,
    );

    get_files_count_recursive(
        &command.sources,
        &command.destination,
        &dir_content_size,
        command.no_progress_bar,
    )?;

    for source in command.sources {
        if Path::new(&source)
            .canonicalize()
            .into_report()
            .change_context(AntigErr)?
            == Path::new(&command.destination)
                .canonicalize()
                .into_report()
                .change_context(AntigErr)?
        {
            continue;
        }

        if Path::new(&source).is_dir() {
            if !command.recursive {
                return Err(Report::new(AntigErr)
                    .attach_printable("cannot copy a directory without recursive process."));
            }
            if !Path::new(&command.destination).is_dir() {
                return Err(Report::new(AntigErr).attach_printable(format!(
                    "`{}` is not a directory.",
                    command.destination.clone()
                )));
            }

            copy_directory_recursive(
                &bar,
                &source,
                &command.destination,
                &dir_content_size,
                command.noise,
                command.no_progress_bar,
            )?;
        } else {
            let destination = if Path::new(&command.destination).is_dir() {
                PathBuf::from(&command.destination).join(&source)
            } else {
                PathBuf::from(&command.destination)
            };
            fs::copy(&source, &destination)
                .into_report()
                .change_context(AntigErr)
                .attach_printable_lazy(|| {
                    format!(
                        "coping failed from `{}` into `{}`.",
                        source,
                        destination.display()
                    )
                })?;
        }
    }

    Ok(())
}

fn visit_dir<const CREATE_DIR: bool>(
    dir: &Path,
    destination: &Path,
    f: &mut dyn FnMut(&DirEntry) -> Result<(), AntigErr>,
    g: Option<&dyn Fn(&DirEntry) -> Result<(), AntigErr>>,
) -> Result<(), AntigErr> {
    if dir.is_dir() {
        for entry in fs::read_dir(dir).into_report().change_context(AntigErr)? {
            let entry = entry.into_report().change_context(AntigErr)?;
            let path = entry.path();
            if path
                .canonicalize()
                .into_report()
                .change_context(AntigErr)
                .attach_printable_lazy(|| {
                    format!("Cannot get the metadata for `{}`", path.display())
                })?
                == destination
                    .canonicalize()
                    .into_report()
                    .change_context(AntigErr)
                    .attach_printable_lazy(|| {
                        format!("Cannot get the metadata for `{}`", destination.display())
                    })?
            {
                continue;
            }

            if path.is_dir() {
                if CREATE_DIR {
                    g.unwrap()(&entry)?;
                }
                visit_dir::<CREATE_DIR>(&path, destination, f, g)?;
            } else {
                f(&entry)?;
            }
        }
    }
    Ok(())
}

fn get_files_count_recursive(
    sources: &[String],
    destination: &str,
    dir_content_size: &Arc<AtomicU64>,
    no_progress_bar: bool,
) -> Result<(), AntigErr> {
    if !no_progress_bar {
        for source in sources {
            if Path::new(source).is_dir() {
                let writer = Arc::clone(&dir_content_size);
                let source_clone = source.to_string();
                let destination_clone = destination.to_string();
                thread::spawn(move || {
                    visit_dir::<false>(
                        &PathBuf::from(source_clone),
                        &PathBuf::from(destination_clone),
                        &mut |_entry| -> Result<(), AntigErr> {
                            writer.fetch_add(1, Ordering::Relaxed);
                            Ok(())
                        },
                        None,
                    )
                    .unwrap();
                });
            }
        }
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
) -> Result<(), AntigErr> {
    let make_destination = PathBuf::from(&destination).join(if Path::new(source).is_absolute() {
        Path::new(source)
            .strip_prefix(Path::new(source).parent().unwrap_or(Path::new("/")))
            .unwrap()
    } else {
        Path::new(source)
    });
    match fs::create_dir(&make_destination) {
        Ok(_) => {}
        Err(err) => match err.kind() {
            std::io::ErrorKind::AlreadyExists => {}
            _ => {
                return Err(Report::new(AntigErr).attach_printable(format!(
                    "Error occurs to create a directory `{}`.\nIOError: {err}",
                    make_destination.display()
                )))
            }
        },
    }

    visit_dir::<true>(
        &PathBuf::from(&source),
        &PathBuf::from(&destination),
        &mut |entry| -> Result<(), AntigErr> {
            let destination = PathBuf::from(&destination).join(
                entry
                    .path()
                    .strip_prefix(Path::new(source).parent().unwrap_or(Path::new("/")))
                    .unwrap(),
            );

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
                        let entry_len = entry
                            .metadata()
                            .into_report()
                            .change_context(AntigErr)
                            .attach_printable_lazy(|| {
                                format!("Cannot get the metadata for `{}`.", entry.path().display())
                            })?
                            .len();
                        let destination_len = PathBuf::from(&destination)
                            .metadata()
                            .into_report()
                            .change_context(AntigErr)
                            .attach_printable_lazy(|| {
                                format!("Cannot get the metadata for `{}`.", destination.display())
                            })?
                            .len();
                        if entry_len != destination_len {
                            fs::remove_file(&destination)
                                .into_report()
                                .change_context(AntigErr)
                                .attach_printable_lazy(|| {
                                    format!("cannot remove `{}`.", destination.display())
                                })?;
                            fs::copy(entry.path(), &destination)
                                .into_report()
                                .change_context(AntigErr)
                                .attach_printable_lazy(|| {
                                    format!(
                                        "coping failed from `{}` into `{}`.",
                                        entry.path().display(),
                                        destination.display()
                                    )
                                })?;
                        }
                    }
                    _ => {
                        return Err(Report::new(AntigErr).attach_printable(format!(
                            "Error occurs to copy from `{}` into `{}`.\nIOError: {err}",
                            entry.path().display(),
                            destination.display()
                        )))
                    }
                },
            }

            if !no_progress_bar {
                bar.inc(1);
            }

            Ok(())
        },
        Some(&|entry| -> Result<(), AntigErr> {
            let destination = PathBuf::from(&destination).join(
                entry
                    .path()
                    .strip_prefix(Path::new(source).parent().unwrap_or(Path::new("/")))
                    .unwrap(),
            );
            match fs::create_dir(&destination) {
                Ok(_) => {}
                Err(err) => match err.kind() {
                    std::io::ErrorKind::AlreadyExists => {}
                    _ => {
                        return Err(Report::new(AntigErr).attach_printable(format!(
                            "Error occurs to create a directory `{}`.\nIOError: {err}",
                            destination.display()
                        )))
                    }
                },
            }
            Ok(())
        }),
    )?;

    Ok(())
}
