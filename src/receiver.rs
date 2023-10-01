use std::{process::ExitStatus, sync::mpsc::Receiver};

use gix::{hash::Kind, ObjectId};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SourceType {
    StdOut,
    StdErr,
}

#[derive(Clone, Debug, PartialEq)]
pub enum WorkloadEvent {
    // TODO Name types would be nice
    Changes(String, ObjectId, ObjectId),
    ActionOutput(String, SourceType, Vec<u8>),
    ActionExit(String, ExitStatus),
    Success(String, ObjectId),
    Failure(String, String, ObjectId),
    Error(String, String, ObjectId),
    Timeout(String),
}

pub fn logging_receiver(events: &Receiver<WorkloadEvent>) {
    while let Ok(event) = events.recv() {
        match event {
            WorkloadEvent::Changes(name, prev_sha, new_sha) => {
                if prev_sha == ObjectId::null(Kind::Sha1) {
                    println!("{}: New repo @ {}", name, new_sha);
                } else {
                    println!("{}: Updated repo {} -> {}", name, prev_sha, new_sha);
                }
            }
            WorkloadEvent::ActionOutput(name, source_type, data) => match source_type {
                SourceType::StdOut => println!("{}: {}", name, String::from_utf8_lossy(&data)),
                SourceType::StdErr => eprintln!("{}: {}", name, String::from_utf8_lossy(&data)),
            },
            WorkloadEvent::ActionExit(name, exit) => {
                println!("{}: exited with code {}", name, exit)
            }
            WorkloadEvent::Success(name, new_sha) => {
                println!("{}: actions successful for {}", name, new_sha)
            }
            WorkloadEvent::Failure(task, action, new_sha) => {
                println!("{}: action {} failed for {}", task, action, new_sha)
            }
            WorkloadEvent::Error(name, error, new_sha) => {
                println!("{}: error running actions for {}: {}", name, new_sha, error)
            }
            WorkloadEvent::Timeout(name) => println!("{}: took too long", name),
        }
    }
}
