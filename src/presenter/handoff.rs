//! Scoped foreground ownership for an explicitly opened reviewer.
use super::*;
use crate::{
    executor::{ProcessGroupGuard, confirm_process_group_gone},
    reviewer::{ReviewLaunch, WaitMode},
};
use std::process::ExitStatus;

struct TerminalOwner {
    #[cfg(unix)]
    saved: libc::termios,
    #[cfg(unix)]
    foreground: i32,
}
impl TerminalOwner {
    fn capture() -> Result<Self> {
        #[cfg(unix)]
        unsafe {
            let mut saved = std::mem::zeroed();
            anyhow::ensure!(
                libc::tcgetattr(0, &mut saved) == 0,
                "cannot save reviewer terminal"
            );
            let foreground = libc::tcgetpgrp(0);
            anyhow::ensure!(foreground > 0, "cannot identify terminal foreground owner");
            Ok(Self { saved, foreground })
        }
        #[cfg(not(unix))]
        Ok(Self {})
    }

    #[cfg(unix)]
    fn transfer(group: i32) -> Result<()> {
        // tcsetpgrp from the background can send SIGTTOU. Mask it on this
        // thread only, restoring the original mask before any await/migration.
        unsafe {
            let mut blocked = std::mem::zeroed();
            let mut previous = std::mem::zeroed();
            libc::sigemptyset(&mut blocked);
            libc::sigaddset(&mut blocked, libc::SIGTTOU);
            anyhow::ensure!(
                libc::pthread_sigmask(libc::SIG_BLOCK, &blocked, &mut previous) == 0,
                "cannot guard terminal ownership transfer"
            );
            let result = libc::tcsetpgrp(0, group);
            let error = io::Error::last_os_error();
            libc::pthread_sigmask(libc::SIG_SETMASK, &previous, std::ptr::null_mut());
            if result != 0 {
                return Err(error.into());
            }
        }
        Ok(())
    }
}
impl Drop for TerminalOwner {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = Self::transfer(self.foreground);
            unsafe {
                libc::tcsetattr(0, libc::TCSANOW, &self.saved);
                libc::tcflush(0, libc::TCIFLUSH);
            }
        }
        let _ = execute!(
            io::stdout(),
            event::DisableBracketedPaste,
            event::DisableMouseCapture,
            terminal::LeaveAlternateScreen,
            crossterm::style::ResetColor,
            crossterm::cursor::Show
        );
    }
}

pub(super) async fn run(ui: &mut Ui, launch: ReviewLaunch) -> Result<ExitStatus> {
    ui.input_boundary().await?;
    let was_alternate = ALTERNATE.load(std::sync::atomic::Ordering::SeqCst);
    ui.screen.take();
    let owner = TerminalOwner::capture()?;
    println!("\n  Review copies only. Changes here are not part of the candidate.");
    if let Some(warning) = &launch.warning {
        println!("  {}", sanitize(warning));
    }
    let mut command = tokio::process::Command::from(launch.command);
    command.kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut group = ProcessGroupGuard::new(None);
    let mut terminated_group = None;
    let result: Result<_> = async {
        let mut child = command.spawn()?;
        group = ProcessGroupGuard::new(child.id());
        #[cfg(unix)]
        if launch.terminal {
            let id = child.id().context("reviewer has no process identity")? as i32;
            TerminalOwner::transfer(id)?;
            // A fast terminal reader may have received SIGTTIN before transfer.
            unsafe { libc::kill(-id, libc::SIGCONT); }
        }
        #[cfg(unix)]
        let status = tokio::select! {
            status = child.wait() => status?,
            _ = ui.term.recv() => { ui.closed = true; terminated_group = group.kill(); let _ = child.kill().await; child.wait().await? },
            _ = ui.hup.recv() => { ui.closed = true; terminated_group = group.kill(); let _ = child.kill().await; child.wait().await? },
            _ = tokio::signal::ctrl_c(), if !launch.terminal => { terminated_group = group.kill(); let _ = child.kill().await; child.wait().await? },
        };
        #[cfg(not(unix))]
        let status = child.wait().await?;
        Ok(status)
    }.await;
    // Regain the terminal before restarting Crossterm. The process-group guard
    // outlives this guard for launchers requiring an explicit document return.
    drop(owner);
    let explicit_return = launch.wait == WaitMode::ExplicitReturn
        && result.as_ref().is_ok_and(|status| status.success())
        && !ui.closed;
    // --wait concerns the reviewed documents, not the user's entire GUI app.
    // Do not close unrelated editor windows after successful document review.
    if !launch.terminal && result.as_ref().is_ok_and(|status| status.success()) {
        group.disarm();
    }
    let killed = if explicit_return { None } else { group.kill() };
    ui.screen = Some(Screen::open(was_alternate)?);
    ui.input_boundary().await?;
    if explicit_return {
        loop {
            match ui.command_prompt("Close the external review document, then press Enter to return.\nReview-copy edits are not imported. Ctrl+C leaves this result pending.").await? {
                Input::Submit(text) if text.trim().is_empty() => break,
                Input::Submit(_) | Input::Changed => {},
                Input::Cancel | Input::Eof => { ui.closed = true; break; },
            }
        }
        ui.input_boundary().await?;
        if !ui.closed {
            group.disarm();
        }
    }
    let killed = group.kill().or(killed).or(terminated_group);
    if let Some(group) = killed
        && !confirm_process_group_gone(group).await
    {
        ui.closed = true;
        anyhow::bail!(
            "reviewer process cleanup could not be confirmed; no acceptance was recorded"
        );
    }
    result
}
