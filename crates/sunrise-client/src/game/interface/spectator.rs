//! The spectator menu (`SpectatorGui`): its players and teams from the
//! server's player list and teams, and the teleport request.
use super::Game;
use sunrise_client::{
    hud::HudBatch,
    hud_layout, keybinds,
    scoreboard::{comp_text, team_color_rgb},
    spectator_menu::{Member, Outcome, SpectatorMenu, Team},
    tab_list::{SPECTATOR, TabPlayer},
    text::Text,
};
use sunrise_protocol::packets::play::{self, Serverbound as S};

impl Game<'_> {
    /// `Util.getMillis` for the menu's fade.
    fn spectator_now(&self) -> i64 {
        self.started.elapsed().as_millis() as i64
    }
    /// `TeleportToPlayerMenuCategory` starts from listed players;
    /// `TeleportToTeamMenuCategory.TeamSelectionItem.create` looks up every team member.
    fn new_spectator_menu(&mut self) -> SpectatorMenu {
        let tab = self
            .remote_tab_players()
            .map_or_else(Vec::new, |(players, _, _)| players);
        let member = |p: &TabPlayer, skin: String| Member {
            uuid: p.uuid,
            name: p.name.clone(),
            skin,
            enabled: p.game_mode != SPECTATOR,
        };
        let players = tab
            .iter()
            .map(|p| member(p, self.face_skin(p.uuid, &p.skin)))
            .collect();
        let teams = &self.score.board.teams;
        let rosters: Vec<_> = self
            .remote
            .iter()
            .flat_map(|remote| teams.iter().map(move |team| (remote, team)))
            .filter_map(|(remote, team)| {
                let members: Vec<_> = team
                    .players
                    .iter()
                    .filter_map(|name| remote.player_info_by_name(name))
                    .filter(|p| p.game_mode != SPECTATOR)
                    .map(|p| member(p, remote.player_info_skin(p.uuid).to_owned()))
                    .collect();
                (!members.is_empty()).then(|| {
                    (
                        comp_text(&team.display_name),
                        team.color.as_deref().and_then(team_color_rgb),
                        members,
                    )
                })
            })
            .collect();
        let teams = rosters
            .into_iter()
            .map(|(name, color, mut players)| {
                for p in &mut players {
                    p.skin = self.face_skin(p.uuid, &p.skin);
                }
                // A random member's face is the team's icon.
                let pick = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.subsec_nanos() as usize)
                    % players.len();
                Team {
                    name,
                    color,
                    icon: players[pick].skin.clone(),
                    players,
                }
            })
            .collect();
        SpectatorMenu::new(players, teams)
    }
    fn spectator_outcome(&mut self, outcome: Outcome) {
        if let Outcome::Teleport(uuid) = outcome {
            // PlayerMenuItem.selectItem.
            self.send(S::TeleportToEntity(play::TeleportToEntity { uuid }));
        }
    }
    /// A hotbar key as a spectator (`SpectatorGui.onHotbarSelected`).
    pub(super) fn spectator_hotbar_key(&mut self, slot: i32) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let outcome = self.spectator_gui.on_hotbar_selected(slot, now, menu);
        self.spectator_outcome(outcome);
    }
    /// `key.spectatorHotbar`: opens the menu, or uses the selected item.
    pub(super) fn spectator_action_key(&mut self) {
        let now = self.spectator_now();
        let menu = self.spectator_menu_if_needed();
        let outcome = self.spectator_gui.on_action_key(now, menu);
        self.spectator_outcome(outcome);
    }
    /// The menu a press would open, built only when none is up.
    fn spectator_menu_if_needed(&mut self) -> Option<SpectatorMenu> {
        (!self.spectator_gui.is_menu_active()).then(|| self.new_spectator_menu())
    }
    /// `MouseHandler.onScroll` in spectator mode with the menu up.
    pub(super) fn spectator_scroll(&mut self, wheel: i32) {
        let now = self.spectator_now();
        self.spectator_gui.on_mouse_scrolled(wheel, now);
    }
    /// `Hud.extractHotbar` for a spectator, and `extractAction`'s prompt.
    pub(super) fn draw_spectator_hotbar(&mut self, batch: &mut HudBatch, show_action: bool) {
        let (w, h) = (batch.width as i32, batch.height as i32);
        let now = self.spectator_now();
        let labels: Vec<Text> = (1..=9)
            .map(|n| keybinds::display_name(self.settings.keys.get(&format!("key.hotbar.{n}"))))
            .collect();
        let font = &self.gui.atlas.font;
        let mut draws = self
            .spectator_gui
            .hotbar(w, h, now, &|slot| labels[slot].clone(), font);
        if show_action {
            draws.extend(self.spectator_gui.action(
                w,
                h,
                now,
                font,
                self.settings.text_background(0.0),
            ));
        }
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{in_own_process, native_entity_refs::tests::with_client_game};
    use sunrise_client::{spectator_menu::Item, tab_list::default_skin};
    use sunrise_protocol::packets::{Empty, play::Clientbound as C};

    #[test]
    #[ignore = "daily"]
    fn an_unlisted_player_is_a_teleport_target_only_on_their_team_page() {
        in_own_process(
            "game::spectator::tests::an_unlisted_player_is_a_teleport_target_only_on_their_team_page",
            || {
                with_client_game(|game| {
                    let (mut peer, _writer) = crate::game::remote::tests::connect_menu_test(game);
                    const UNLISTED: u128 = 0xcafe;
                    const LISTED: u128 = 0xbeef;
                    game.handle_packet(C::PlayerInfoUpdate(play::PlayerInfoUpdate {
                        actions: 0x01 | 0x04 | 0x08, // ADD_PLAYER | UPDATE_GAME_MODE | UPDATE_LISTED
                        entries: [
                            (UNLISTED, "Unlisted", 0, false),
                            (LISTED, "Listed", 0, true),
                            (0xaa, "Spectator", SPECTATOR, false),
                        ]
                        .into_iter()
                        .map(|(uuid, name, game_mode, listed)| play::PlayerInfo {
                            uuid,
                            name: Some(name.into()),
                            game_mode: Some(game_mode),
                            listed: Some(listed),
                            ..play::PlayerInfo::default()
                        })
                        .collect(),
                    }))
                    .unwrap();
                    let board = &mut game.score.board;
                    board.add_team("hidden");
                    for name in ["Unlisted", "Spectator", "Unknown"] {
                        board.add_to_team(name, "hidden");
                    }

                    let mut players = game.new_spectator_menu();
                    players.select_slot(0);
                    players.select_slot(0);
                    assert!(matches!(players.item(0), Item::Player(p) if p.uuid == LISTED));
                    assert_eq!(players.item(1), Item::Empty);

                    let mut teams = game.new_spectator_menu();
                    assert_eq!(teams.item(1), Item::TeleportToTeam { enabled: true });
                    teams.select_slot(1);
                    teams.select_slot(1);
                    let Item::Team(team) = teams.item(0) else {
                        panic!("team with only an unlisted eligible member is missing");
                    };
                    assert_eq!(team.icon, default_skin(UNLISTED));
                    teams.select_slot(0);
                    teams.select_slot(0);
                    assert!(matches!(teams.item(0), Item::Player(p) if p.uuid == UNLISTED));
                    assert_eq!(teams.item(1), Item::Empty);
                    assert_eq!(teams.select_slot(0), Outcome::None);
                    game.spectator_outcome(teams.select_slot(0));
                    game.send(S::ClientTickEnd(Empty));
                    let mut teleports = Vec::new();
                    loop {
                        let (id, body) = peer.recv().unwrap();
                        match S::decode(id, &body).unwrap() {
                            S::TeleportToEntity(p) => teleports.push(p.uuid),
                            S::ClientTickEnd(_) => break,
                            _ => {}
                        }
                    }
                    assert_eq!(teleports, [UNLISTED]);
                    let (tab, _, _) = game.remote_tab_players().unwrap();
                    assert_eq!(tab.iter().map(|p| p.uuid).collect::<Vec<_>>(), [LISTED]);
                });
            },
        );
    }
}
