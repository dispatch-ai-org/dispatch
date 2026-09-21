//! CLI and in-session setup use this same prompt/controller and shared service.
use super::*;
use crate::setup::{self, Proposal};

pub async fn standalone(
    state: &State,
    provider: Option<String>,
    project_checks: bool,
    mut options: Options,
) -> Result<()> {
    anyhow::ensure!(
        suitable(),
        "setup requires a terminal for explicit funding confirmation; use dispatch resources for noninteractive status"
    );
    options.plain |= std::env::var("TERM").is_ok_and(|v| v == "dumb");
    let mut ui = Ui::new(options)?;
    if project_checks {
        checks(&mut ui, &std::env::current_dir()?).await
    } else {
        accounts(&mut ui, state, provider).await
    }
}

pub(super) async fn accounts(
    ui: &mut Ui,
    state: &State,
    mut selected: Option<String>,
) -> Result<()> {
    loop {
        let status = setup::status(state)?;
        let choice = if let Some(provider) = selected.take() {
            provider
        } else {
            match ui.command_prompt(&format!("{status}\n\n[c] Add Codex   [a] Add Claude\n[number] Revalidate profile   [l] Provider login   [b] Back")).await? {
                Input::Submit(v)=>v.trim().to_lowercase(), _=>return Ok(()),
            }
        };
        if matches!(choice.as_str(), "b" | "back" | "") {
            return Ok(());
        }
        if choice == "l" {
            let provider=match ui.command_prompt("Provider login changes the provider's saved account.\nChoose codex or claude, or cancel.").await? {
                Input::Submit(v) if matches!(v.trim(),"codex"|"claude")=>v.trim().to_owned(), _=>continue,
            };
            if let Err(e) = login(ui, &provider).await {
                ui.commit(&format!(
                    "Login stopped: {e}. No Dispatch funding authorization changed."
                ))?;
            }
            continue;
        }
        let resources = crate::config::ResourceConfig::load(&state.root)?;
        let (provider, index) = match choice.as_str() {
            "c" | "codex" => ("codex".to_owned(), None),
            "a" | "claude" => ("claude".to_owned(), None),
            value => match value
                .parse::<usize>()
                .ok()
                .and_then(|n| n.checked_sub(1))
                .filter(|n| *n < resources.profiles.len())
            {
                Some(i) => (resources.profiles[i].harness.clone(), Some(i)),
                None => {
                    ui.commit("Choose a listed resource or provider.")?;
                    continue;
                }
            },
        };
        if let Err(e) = configure(ui, state, &provider, index).await {
            ui.commit(&format!("Resource unchanged: {}\nChoose Login for missing authentication, or revalidate after correcting provider settings.",clip(&format!("{e:#}"),500)))?;
        }
    }
}

async fn configure(ui: &mut Ui, state: &State, provider: &str, index: Option<usize>) -> Result<()> {
    let executable = setup::executable(provider)?;
    let discovery = ui.discover(provider, executable).await?;
    let Some(discovery) = discovery else {
        return Ok(());
    };
    let (model, effort, tier) = if index.is_some() {
        (String::new(), String::new(), ResourceTier::Standard)
    } else {
        let default_model = if provider == "codex" {
            "gpt-5.6-sol"
        } else {
            "claude-sonnet-5"
        };
        let Input::Submit(model)=ui.command_prompt(&format!("{}\n\nExact included model ID [{default_model}]\nConfirm inclusion with your provider; a default is not evidence.",discovery.summary())).await? else{return Ok(())};
        let model = if model.trim().is_empty() {
            default_model.into()
        } else {
            model.trim().into()
        };
        let Input::Submit(effort) = ui
            .command_prompt("Effort [medium] · low / medium / high / xhigh")
            .await?
        else {
            return Ok(());
        };
        let effort = if effort.trim().is_empty() {
            "medium".into()
        } else {
            effort.trim().into()
        };
        let Input::Submit(tier)=ui.command_prompt("Resource tier [standard] · light / standard / strong\nChoose the resource's capability tier. This does not change task suitability rules.").await? else{return Ok(())};
        let tier = match tier.trim() {
            "light" => ResourceTier::Light,
            "strong" => ResourceTier::Strong,
            "standard" | "" => ResourceTier::Standard,
            _ => anyhow::bail!("unknown tier"),
        };
        (model, effort, tier)
    };
    let proposal = Proposal::prepare(state, discovery, provider, index, model, effort, tier)?;
    // Longer consent content uses scrollback plus a short explicit command prompt.
    ui.commit(&proposal.summary())?;
    if matches!(ui.command_prompt("Authorize this resource? Type confirm, or cancel.").await?,Input::Submit(v) if v.trim()=="confirm")
    {
        proposal.confirm(state)?;
        ui.commit("Resource saved. Current account, funding and shared admission are checked again before launch.")?;
    } else {
        ui.commit("Revalidation cancelled. Configuration unchanged.")?;
    }
    Ok(())
}

impl Ui {
    async fn discover(
        &mut self,
        provider: &str,
        path: std::path::PathBuf,
    ) -> Result<Option<setup::Discovery>> {
        let future = setup::discover(provider, path);
        tokio::pin!(future);
        self.commit("Checking installed CLI and supported authentication · no model call")?;
        loop {
            self.draw(
                "Accounts / Resources\nChecking authentication…",
                None,
                "Ctrl+C cancel · no model call",
            )?;
            tokio::select! {
                result=&mut future=>return result.map(Some),
                event=self.next()=>match event? {
                    None=>{self.closed=true;return Ok(None)},
                    Some(Event::Key(k)) if k.code==KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL)=>return Ok(None),
                    _=>{}
                }
            }
        }
    }
}

async fn login(ui: &mut Ui, provider: &str) -> Result<()> {
    let executable = setup::executable(provider)?;
    let Input::Submit(answer)=ui.command_prompt(&format!("Open {provider}'s supported login? This may open a browser and change its saved account.\nNo funding is authorized by login. Type login to continue.")).await? else{return Ok(())};
    if answer.trim() != "login" {
        return Ok(());
    }
    let temp = tempfile::tempdir()?;
    let mut command = std::process::Command::new(executable);
    command.current_dir(temp.path()).env_clear();
    for key in ["HOME", "USER", "PATH", "TERM", "TMPDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    if provider == "codex" {
        command.args(["login", "--device-auth"]);
    } else {
        command.args(["auth", "login"]);
    }
    let status = handoff::run_named(
        ui,
        crate::reviewer::ReviewLaunch {
            command,
            wait: crate::reviewer::WaitMode::Process,
            terminal: true,
            warning: None,
        },
        "Provider login · return here to explicitly validate funding.",
    )
    .await?;
    anyhow::ensure!(status.success(), "provider login did not complete");
    ui.commit("Login returned. Choose a resource to revalidate its funding.")
}

pub(super) async fn checks(ui: &mut Ui, source: &std::path::Path) -> Result<()> {
    let expected = setup::project_config_bytes(source)?;
    let choices = setup::check_choices(source);
    let list = choices
        .iter()
        .enumerate()
        .map(|(i, c)| format!("[{}] {c}", i + 1))
        .collect::<Vec<_>>()
        .join("\n");
    let Input::Submit(answer)=ui.command_prompt(&format!("Choose checks\n{list}\n[n] Continue unverified   [b] Back\nA check runs project code with your permissions. Choosing a command approves it for this project; it does not prove task-specific correctness.")).await? else{anyhow::bail!("check selection cancelled")};
    if answer.trim() == "n" {
        return Ok(());
    }
    let command = answer
        .trim()
        .parse::<usize>()
        .ok()
        .and_then(|n| n.checked_sub(1))
        .and_then(|n| choices.get(n))
        .context("check selection cancelled")?;
    setup::save_checks(source, command, expected)?;
    ui.commit(&format!("Approved check saved: {command}"))
}
