//! Work that outlives a window: the speech and cleanup models' downloads,
//! started from the onboarding. Each runs on a thread of its own and
//! reports its progress here, and closing the window doesn't stop it: the
//! app quits once the last one finishes.

use std::collections::HashMap;

use gpui_kit::*;

#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    /// Bytes done of total (total 0 until it's known), and what it's doing
    /// ("" for one-step work).
    Running(u64, u64, &'static str),
    Done,
    Failed(String),
}

impl Job {
    /// How far along, 0..1.
    pub fn fraction(&self) -> f32 {
        match self {
            Job::Running(done, total, _) if *total > 0 => (*done as f64 / *total as f64) as f32,
            Job::Running(..) | Job::Failed(_) => 0.,
            Job::Done => 1.,
        }
    }
}

#[derive(Default)]
pub struct Jobs {
    jobs: HashMap<String, Job>,
    /// The window closed while a job ran: quit once they're all done.
    quit_when_idle: bool,
}

impl Global for Jobs {}

impl Jobs {
    pub fn init(cx: &mut App) {
        cx.set_global(Jobs::default());
    }

    pub fn get(key: &str, cx: &App) -> Option<Job> {
        cx.global::<Jobs>().jobs.get(key).cloned()
    }

    pub fn busy(cx: &App) -> bool {
        cx.global::<Jobs>().jobs.values().any(|j| matches!(j, Job::Running(..)))
    }

    /// Quit now, or once the running jobs are done.
    pub fn quit_when_idle(cx: &mut App) {
        if Jobs::busy(cx) {
            cx.global_mut::<Jobs>().quit_when_idle = true;
        } else {
            cx.quit();
        }
    }

    /// Run `work` under `key`, unless it's already running. It reports
    /// progress as (done, total).
    pub fn start(
        key: impl Into<String>,
        cx: &mut App,
        work: impl FnOnce(&mut dyn FnMut(u64, u64)) -> Result<(), String> + Send + 'static,
    ) {
        Jobs::start_staged(key, cx, move |progress| work(&mut |done, total| progress("", done, total)));
    }

    /// The same for work in steps: it reports (step, done, total), the
    /// progress counting afresh in each step.
    pub fn start_staged(
        key: impl Into<String>,
        cx: &mut App,
        work: impl FnOnce(&mut dyn FnMut(&'static str, u64, u64)) -> Result<(), String> + Send + 'static,
    ) {
        let key = key.into();
        if matches!(Jobs::get(&key, cx), Some(Job::Running(..))) {
            return;
        }
        let (tx, rx) = async_channel::unbounded::<Job>();
        std::thread::spawn(move || {
            let mut last = ("", u64::MAX);
            let progress_tx = tx.clone();
            let mut progress = move |stage: &'static str, done: u64, total: u64| {
                // A few hundred updates at most, not one per chunk.
                let step = (stage, (done * 400).checked_div(total).unwrap_or(done >> 20));
                if step != last {
                    last = step;
                    let _ = progress_tx.send_blocking(Job::Running(done, total, stage));
                }
            };
            let end = match work(&mut progress) {
                Ok(()) => Job::Done,
                Err(e) => Job::Failed(e),
            };
            let _ = tx.send_blocking(end);
        });
        cx.update_global::<Jobs, _>(|jobs, _| {
            jobs.jobs.insert(key.clone(), Job::Running(0, 0, ""));
        });
        cx.spawn(async move |cx| {
            while let Ok(job) = rx.recv().await {
                let finished = !matches!(job, Job::Running(..));
                cx.update(|cx| {
                    cx.update_global::<Jobs, _>(|jobs, _| {
                        jobs.jobs.insert(key.clone(), job);
                    });
                    if finished && cx.global::<Jobs>().quit_when_idle && !Jobs::busy(cx) {
                        cx.quit();
                    }
                });
                if finished {
                    break;
                }
            }
        })
        .detach();
    }
}
