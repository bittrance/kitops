use std::{process::ExitStatus, sync::mpsc::Receiver};

use gix::{hash::Kind, ObjectId};

#[derive(Clone, Copy)]
pub enum SourceType {
    StdOut,
    StdErr,
}

pub enum ActionOutput {
    Changes(String, ObjectId, ObjectId),
    Output(String, SourceType, Vec<u8>),
    Exit(String, ExitStatus),
    Timeout(String),
}

pub fn logging_receiver(events: &Receiver<ActionOutput>) {
    while let Ok(event) = events.recv() {
        match event {
            ActionOutput::Changes(name, prev_sha, new_sha) => {
                if prev_sha == ObjectId::null(Kind::Sha1) {
                    println!("{}: New repo @ {}", name, new_sha);
                } else {
                    println!("{}: Updated repo {} -> {}", name, prev_sha, new_sha);
                }
            }
            ActionOutput::Output(name, source_type, data) => match source_type {
                SourceType::StdOut => println!("{}: {}", name, String::from_utf8_lossy(&data)),
                SourceType::StdErr => eprintln!("{}: {}", name, String::from_utf8_lossy(&data)),
            },
            ActionOutput::Exit(name, exit) => println!("{}: exited with code {}", name, exit),
            ActionOutput::Timeout(name) => println!("{}: took too long", name),
        }
    }
}
