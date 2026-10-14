use super::*;
use crate::api::schema::Method;

fn agent(pane_id: &str, tab_id: &str) -> ClientShellAgent {
    ClientShellAgent {
        pane_id: pane_id.into(),
        workspace_id: "ws_1".into(),
        tab_id: tab_id.into(),
        name: None,
        display_agent: None,
        agent: Some("claude".into()),
        title: None,
        terminal_title: None,
        terminal_title_stripped: None,
        agent_status: AgentStatus::Working,
        state_change_seq: 1,
        state_labels: Vec::new(),
        tokens: Vec::new(),
        focused: pane_id == "pane_1",
    }
}

/// The default snapshot (one workspace, one tab, one pane), plus `extra_tabs`
/// pane-less tabs and an agent on pane_1 when `with_agent` is set.
fn shell_state(
    config: Config,
    confirm: bool,
    with_agent: bool,
    extra_tabs: usize,
) -> ClientShellState {
    let mut projected = snapshot();
    for number in 2..2 + extra_tabs {
        let mut tab = projected.tabs[0].clone();
        tab.tab_id = format!("tab_{number}");
        tab.number = number;
        tab.focused = false;
        projected.tabs.push(tab);
    }
    if with_agent {
        projected.agents.push(agent("pane_1", "tab_1"));
    }
    let mut state = ClientShellState::new(ClientShellConfig::from_config(&config));
    state.config.confirm_close = confirm;
    state.set_snapshot(Box::new(projected));
    state.set_pane_surface(surface());
    state.compose(106, 24).unwrap();
    state
}

/// Two tabs in one workspace so closing tab_1 never closes the workspace.
fn agent_state(config: Config, confirm: bool, with_agent: bool) -> ClientShellState {
    shell_state(config, confirm, with_agent, 1)
}

fn bind(state: &mut ClientShellState, action: crate::input::KeybindAction) -> ClientShellInput {
    let mut outcome = ClientShellInput::default();
    state.record_binding(crate::input::KeybindMatch::Action(action), &mut outcome);
    outcome
}

fn close_methods(outcome: &ClientShellInput) -> Vec<&Method> {
    outcome
        .actions
        .iter()
        .filter_map(|action| match action {
            ClientShellAction::Endpoint { request, .. } => Some(&request.method),
            _ => None,
        })
        .filter(|method| {
            matches!(
                method,
                Method::PaneClose(_) | Method::TabClose(_) | Method::WorkspaceClose(_)
            )
        })
        .collect()
}

fn is_pane_close(outcome: &ClientShellInput) -> bool {
    matches!(close_methods(outcome).as_slice(),
        [Method::PaneClose(target)] if target.pane_id == "pane_1")
}

fn is_tab_close(outcome: &ClientShellInput) -> bool {
    matches!(close_methods(outcome).as_slice(),
        [Method::TabClose(target)] if target.tab_id == "tab_1")
}

#[test]
fn closing_an_agent_pane_waits_for_confirmation() {
    let mut state = agent_state(Config::default(), true, true);
    let requested = bind(&mut state, crate::input::KeybindAction::ClosePane);
    assert!(close_methods(&requested).is_empty());
    assert!(requested.repaint);
    let frame = state.compose(106, 24).unwrap();
    let text = frame_rows(&frame).join("\n");
    assert!(text.contains("Close pane?"));
    assert!(text.contains("stops claude (working)"));

    assert!(is_pane_close(&state.handle_input_bytes(b"\r")));
    assert!(state.overlay.is_none());
}

#[test]
fn agent_pane_close_confirmation_can_be_cancelled() {
    let mut state = agent_state(Config::default(), true, true);
    bind(&mut state, crate::input::KeybindAction::ClosePane);
    assert!(close_methods(&state.handle_input_bytes(b"\x1b")).is_empty());
    assert!(state.overlay.is_none());
}

#[test]
fn pane_close_is_immediate_without_agent_or_with_confirmation_disabled() {
    for (confirm, with_agent) in [(true, false), (false, true), (false, false)] {
        let mut state = agent_state(Config::default(), confirm, with_agent);
        assert!(is_pane_close(&bind(
            &mut state,
            crate::input::KeybindAction::ClosePane
        )));
        assert!(state.overlay.is_none());
    }
}

#[test]
fn closing_a_tab_with_agents_waits_for_confirmation() {
    let mut state = agent_state(Config::default(), true, true);
    assert!(close_methods(&bind(&mut state, crate::input::KeybindAction::CloseTab)).is_empty());
    let frame = state.compose(106, 24).unwrap();
    assert!(frame_rows(&frame).join("\n").contains("Close tab?"));
    assert!(is_tab_close(&state.handle_input_bytes(b"\r")));

    let mut state = agent_state(Config::default(), true, false);
    assert!(is_tab_close(&bind(
        &mut state,
        crate::input::KeybindAction::CloseTab
    )));
}

#[test]
fn agent_close_confirmation_rejects_a_pane_that_disappeared() {
    let mut state = agent_state(Config::default(), true, true);
    bind(&mut state, crate::input::KeybindAction::ClosePane);
    let mut projected = state.snapshot.as_deref().unwrap().clone();
    projected.panes.clear();
    projected.agents.clear();
    state.set_snapshot(Box::new(projected));
    assert!(close_methods(&state.handle_input_bytes(b"\r")).is_empty());
    assert!(state.overlay.is_none());
}

#[test]
fn navigate_mode_close_key_confirms_before_stopping_an_agent() {
    let mut config = Config::default();
    config.ui.modal_navigation = true;
    let mut state = agent_state(config, true, true);
    assert_eq!(state.mode, ClientShellMode::Navigate);
    assert!(close_methods(&state.handle_input_bytes(b"x")).is_empty());
    assert!(matches!(
        state.overlay,
        Some(ClientShellOverlay::ConfirmClose(_))
    ));
    assert!(is_pane_close(&state.handle_input_bytes(b"\r")));
}

#[test]
fn cancelling_an_agent_close_returns_to_the_prior_mode() {
    let mut state = agent_state(Config::default(), true, true);
    assert_eq!(state.mode, ClientShellMode::Terminal);
    bind(&mut state, crate::input::KeybindAction::ClosePane);
    state.handle_input_bytes(b"\x1b");
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.navigate_workspace_id.is_none());
}

/// ws_1 is a worktree parent with one tab and one agent pane; ws_2 is its
/// linked child, so closing ws_1's last tab or pane closes the group.
fn worktree_group_state() -> ClientShellState {
    let mut state = agent_state(Config::default(), true, true);
    let mut projected = state.snapshot.as_deref().unwrap().clone();
    projected.workspaces[0].worktree = Some(ClientShellWorktree {
        key: "repo".into(),
        label: "repo".into(),
        is_linked_worktree: false,
    });
    let mut child = projected.workspaces[0].clone();
    child.workspace_id = "ws_2".into();
    child.active_tab_id = "tab_2".into();
    child.focused = false;
    child.worktree.as_mut().unwrap().is_linked_worktree = true;
    projected.workspaces.push(child);
    projected.tabs[1].workspace_id = "ws_2".into();
    state.set_snapshot(Box::new(projected));
    state
}

#[test]
/// Closing a worktree group parent's last tab or pane asks once, naming the
/// agents it stops, and confirms with an explicit group close so the server
/// does not ask again.
fn worktree_group_closes_confirm_once_with_agents() {
    for action in [
        crate::input::KeybindAction::ClosePane,
        crate::input::KeybindAction::CloseTab,
    ] {
        let mut state = worktree_group_state();

        assert!(close_methods(&bind(&mut state, action)).is_empty());
        assert!(matches!(state.overlay.as_ref(),
            Some(ClientShellOverlay::ConfirmClose(confirm))
                if confirm.title == "Close worktree group?"
                    && confirm.detail.contains("stops claude (working)")));
        let accepted = state.handle_input_bytes(b"\r");
        assert!(matches!(close_methods(&accepted).as_slice(),
            [Method::WorkspaceClose(params)]
                if params.workspace_id == "ws_1" && params.close_group));
        assert!(state.overlay.is_none());
    }
}

#[test]
fn agent_close_confirmation_names_the_agent_like_the_sidebar() {
    let mut state = agent_state(Config::default(), true, false);
    let mut projected = state.snapshot.as_deref().unwrap().clone();
    let mut named = agent("pane_1", "tab_1");
    named.name = Some("reviewer".into());
    named.agent_status = AgentStatus::Blocked;
    projected.agents.push(named);
    state.set_snapshot(Box::new(projected));
    bind(&mut state, crate::input::KeybindAction::ClosePane);
    let frame = state.compose(106, 24).unwrap();
    assert!(frame_rows(&frame)
        .join("\n")
        .contains("stops reviewer (blocked)"));
}

/// One tab with one pane: closing that pane closes the workspace.
fn last_pane_state(confirm: bool, with_agent: bool) -> ClientShellState {
    shell_state(Config::default(), confirm, with_agent, 0)
}

#[test]
fn closing_a_workspace_last_pane_confirms_the_workspace_close() {
    for with_agent in [false, true] {
        let mut state = last_pane_state(true, with_agent);
        assert!(
            close_methods(&bind(&mut state, crate::input::KeybindAction::ClosePane)).is_empty()
        );
        let frame = state.compose(106, 24).unwrap();
        let text = frame_rows(&frame).join("\n");
        assert!(text.contains("Close workspace?"));
        assert!(text.contains("1 pane"));
        assert_eq!(text.contains("stops claude (working)"), with_agent);

        assert!(is_pane_close(&state.handle_input_bytes(b"\r")));
        assert!(state.overlay.is_none());
    }
}

#[test]
fn closing_a_workspace_last_pane_is_immediate_with_confirmation_disabled() {
    let mut state = last_pane_state(false, true);
    assert!(is_pane_close(&bind(
        &mut state,
        crate::input::KeybindAction::ClosePane
    )));
    assert!(state.overlay.is_none());
}

#[test]
fn cancelling_a_last_pane_close_keeps_the_workspace() {
    let mut state = last_pane_state(true, false);
    assert_eq!(state.mode, ClientShellMode::Terminal);
    bind(&mut state, crate::input::KeybindAction::ClosePane);
    assert!(close_methods(&state.handle_input_bytes(b"\x1b")).is_empty());
    assert!(state.overlay.is_none());
    assert_eq!(state.mode, ClientShellMode::Terminal);
    assert!(state.navigate_workspace_id.is_none());
}

#[test]
fn implied_group_close_is_rechecked_when_confirmed() {
    let mut state = worktree_group_state();
    bind(&mut state, crate::input::KeybindAction::ClosePane);
    assert!(matches!(state.overlay.as_ref(),
        Some(ClientShellOverlay::ConfirmClose(confirm)) if confirm.title == "Close worktree group?"));

    // A pane added before confirming means closing pane_1 no longer closes
    // the workspace, so the group close must not go through.
    let mut projected = state.snapshot.as_deref().unwrap().clone();
    let mut split = projected.panes[0].clone();
    split.pane_id = "pane_9".into();
    split.focused = false;
    projected.panes.push(split);
    state.set_snapshot(Box::new(projected));

    assert!(close_methods(&state.handle_input_bytes(b"\r")).is_empty());
    assert!(state.overlay.is_none());
}
