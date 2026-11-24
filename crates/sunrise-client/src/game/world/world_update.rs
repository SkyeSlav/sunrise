//! The frame's world update (`update_world`): what arrived from the server, the chunks that
//! streamed in and the meshes handed to the renderer, the columns that unload (their entities
//! and block entities saved out first).
use super::*;

    pub(super) fn update_world(&mut self) {
        if let Err(e) = sunrise_client::spans::time("poll_remote", || self.poll_remote()) {
            log::warn!("Sunrise: {e}");
        }
        if let Err(e) = self.advance_studio_replay() {
            log::warn!("Sunrise Studio replay: {e}");
        }
        self.poll_remote_light();
        // The renderer laid translucent quads out again (a shader pack's flat terrain came
        // or went) and dropped every section: meshed again, as for a new one.
        if self
            .renderer
            .as_mut()
            .is_some_and(Renderer::take_dropped_sections)
        {
            self.all_sections_changed();
        }
        self.observe_light_edits();
        self.ask_edited_light_emission();
        let camera_pos = self.visual_camera_position();
        if let Some(pos) = camera_pos {
            self.streaming.center = ((pos[0].floor() as i32) >> 4, (pos[2].floor() as i32) >> 4);
            self.streaming.center_y = pos[1].floor() as i32;
        }
        sunrise_client::spans::time("tick_pregen", || self.tick_pregen());
        // A headless game has nothing to draw sections with: like a server's, they aren't
        // meshed while it has no renderer.
        if self.options.headless {
            self.streaming.mesh = self.renderer.is_some();
        }
        if self.defer_chunk_disk_save() {
            return;
        }
        let areas = self.streaming.extra.len();
        // The configuration connection is not a guest and owns no player area.
        // `extra` finishes chunks through radius + 1, even with no players.
        self.streaming.extra.extend(
            self.tickets
                .spawn_areas()
                .map(|(center, radius)| (center, radius - 1)),
        );
        let dim = self.dim;
        if let Some(host) = &self.lan {
            self.streaming.extra.extend(
                host.hub
                    .lan
                    .configuration_tasks
                    .loading_spawns(dim.id())
                    .filter(|&center| !self.tickets.holds_player_spawn(center))
                    .map(|center| {
                        (
                            center,
                            sunrise_client::ticket_levels::PLAYER_SPAWN_RADIUS - 1,
                        )
                    }),
            );
        }
        self.streaming.extra.extend(
            self.server_trips
                .iter()
                .flat_map(|trip| trip.spawn_destination_areas(dim)),
        );
        let waiting = std::mem::take(&mut self.spawn_wait);
        self.streaming
            .extra
            .extend(waiting.into_iter().map(|chunk| (chunk, 0)));
        // `Level.getChunk`'s tickets keep their columns until they expire.
        self.streaming.hold_unknown_tickets();
        self.streaming.upload_backlog = self.pending_uploads.len();
        let plan = sunrise_client::spans::time("streaming_plan", || {
            let mut step = self.streaming.begin_plan_update(&mut self.world);
            loop {
                match step {
                    sunrise_client::streaming::PlanStep::Ready(plan) => break plan,
                    sunrise_client::streaming::PlanStep::Save { chunk, resume } => {
                        self.native_late_full_chunk_snapshot(&chunk);
                        self.streaming.save_finished(chunk);
                        step = self.streaming.resume_plan_update(&mut self.world, resume);
                    }
                }
            }
        });
        let (commit, protos) = self.streaming.prepare_commit(&mut self.world, plan);
        self.native_proto_snapshots(&protos);
        self.streaming.submit_protos(protos);
        let (mut uploads, unloaded) = sunrise_client::spans::time("streaming", || {
            self.streaming.complete_commit(&mut self.world, commit)
        });
        // Sections chosen to mesh wait for the mods' `section_visibility` (none without a mod).
        self.prepare_section_visibility();
        self.streaming.extra.truncate(areas);
        // The viewed levels' meshing, queued after the player's level's (nothing while no view
        // names one).
        if self.streaming.mesh
            && let Some(remote) = &mut self.remote
        {
            let streaming = &self.streaming;
            remote
                .viewed
                .mesh_frame(|| streaming.viewed_mesh_source(), std::time::Instant::now());
        }
        let dirty: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        let modified = sunrise_client::spans::time("with_entities", || self.with_entities(&dirty));
        self.native_full_chunk_snapshots(&modified);
        if !modified.is_empty()
            && let Err(e) = sunrise_client::spans::time("save_chunks", || {
                self.save
                    .save_later(&modified, self.pack.assets.registry, world_biomes!(self))
            })
        {
            log::warn!("Sunrise: failed to save unloaded chunks: {e}");
        }
        sunrise_client::spans::time("spawn_fresh_chunks", || self.spawn_fresh_chunks());
        self.ask_loaded_light_emission();
        // Vanilla saves the chunk before its unload hook and drops its block entities afterwards.
        sunrise_client::spans::time("mods_chunks_changed", || {
            self.native_chunks_loaded();
            self.native_chunks_unloaded(&unloaded);
        });
        for chunk in &unloaded {
            self.world.modified.unloaded(&(chunk.x, chunk.z));
        }
        sunrise_client::spans::time("unload_entities", || self.unload_entities(&unloaded));
        if !unloaded.is_empty() {
            let gone: sunrise_client::fast_hash::FxHashSet<(i32, i32)> =
                unloaded.iter().map(|c| (c.x, c.z)).collect();
            sunrise_client::spans::time("unload_block_entities", || {
                for &column in &gone {
                    self.world.block_entities.unload_column(column);
                }
            });
            self.world
                .block_names
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
        }
        // LevelChunk.registerTickContainerInLevel: a chunk that became
        // resident has its tick containers before a command can schedule into
        // it, even in a frozen level that runs no world tick. The unloaded
        // chunks above were saved with their ticks first.
        if self.remote.is_none() {
            sunrise_client::spans::time("sync_tick_chunks", || self.sync_tick_chunks());
        }
        // Terrain still on its way from the server (single player's integrated one
        // too): the view's outermost columns not all in, or its events not all handled.
        let radius = match &self.remote {
            // As far as the server fills it: its view distance and one.
            Some(r) => self.streaming.radius.min(r.view_distance + 1),
            None => self.streaming.radius,
        };
        let view = (self.streaming.center, radius);
        let has = |column: (i32, i32)| self.world.chunks.contains_key(&column);
        let now = Instant::now();
        let arriving = {
            let _span = sunrise_client::spans::guard("view_arrival");
            self.view_arrival.arriving(view, now, has) || self.remote_backlog() > 0
        };
        if let Some(renderer) = self.renderer.as_mut() {
            let g = &self.settings.sunrise;
            let budgeted = g.chunk_updates > 0 || g.upload_budget_kib > 0;
            let mut raw_flight = self.flight.as_mut().filter(|flight| flight.raw);
            let vertex_bytes = |mesh: &sunrise_client::mesher::SectionMesh| {
                ((mesh.solid.len() + mesh.translucent.len())
                    * std::mem::size_of::<sunrise_client::mesher::TerrainVertex>())
                    as u64
            };
            // The meshes, once copied, are freed by a worker (`Streaming::free`).
            if !budgeted && self.pending_uploads.is_empty() && uploads.len() <= AUTO_UPLOADS_AT_ONCE
            {
                sunrise_client::spans::time("upload", || {
                    for upload in &uploads {
                        if self.streaming.can_publish_mesh(&self.world, upload) {
                            renderer.upload(upload.key, &upload.mesh);
                            if let Some(flight) = raw_flight.as_deref_mut() {
                                flight.note_mesh_uploads(1, vertex_bytes(&upload.mesh));
                            }
                        }
                    }
                });
                self.streaming.free(uploads);
            } else {
                // Sunrise's chunk-update and upload budgets: the nearest
                // sections go first, the rest wait for later frames; a newer
                // mesh of a waiting section replaces it, and sections of
                // chunks no longer loaded are dropped.
                let pending = &mut self.pending_uploads;
                let order_span = sunrise_client::spans::guard("order_uploads");
                let fresh: sunrise_client::fast_hash::FxHashSet<(i32, i32, i32)> =
                    uploads.iter().map(|upload| upload.key).collect();
                let current = |upload: &sunrise_client::streaming::MeshUpload| {
                    self.world
                        .chunks
                        .contains_key(&(upload.key.0, upload.key.2))
                        && self.streaming.can_publish_mesh(&self.world, upload)
                };
                // Filtering leaves the surviving waiting run in the same order.
                let mut spent: Vec<_> = pending
                    .extract_if(.., |upload| fresh.contains(&upload.key) || !current(upload))
                    .collect();
                spent.extend(uploads.extract_if(.., |upload| !current(upload)));
                let centre = self.streaming.center;
                // Section keys count from the world's bottom (`origin`'s min_y + y * 16).
                let bottom = f64::from(self.world.bottom_y());
                let cam_y = camera_pos.map_or(4, |pos| ((pos[1] - bottom) / 16.0).floor() as i32);
                let far = |k: &(i32, i32, i32)| section_distance_sq(*k, centre, cam_y);
                let order = Some((centre, cam_y));
                let priority = |upload: &sunrise_client::streaming::MeshUpload| {
                    std::cmp::Reverse(far(&upload.key))
                };
                let work = order_uploads(
                    pending,
                    &mut uploads,
                    &mut self.pending_uploads_merge,
                    self.pending_uploads_order == order,
                    priority,
                );
                self.pending_uploads_order = order;
                drop(order_span);
                let (limit, budget) = if budgeted {
                    (
                        g.chunk_updates as usize,
                        u64::from(g.upload_budget_kib) * 1024,
                    )
                } else {
                    (0, 0)
                };
                let (mut count, mut bytes) = (0, 0u64);
                // Without the player's budgets, uploads stop once they took this
                // frame's share (`UploadPace::budget`: a quarter of the recent frame
                // time, more with a backlog, at most 2 ms; at least one goes): a burst
                // of finished meshes spreads over frames, nearest first.
                let auto = (!budgeted).then(|| self.upload_pace.budget(pending.len(), arriving));
                let started = Instant::now();
                let upload_span = sunrise_client::spans::guard("upload_queue");
                while let Some(upload) = pending.last() {
                    let mesh = &upload.mesh;
                    if auto.is_some_and(|time| count > 0 && started.elapsed() >= time) {
                        break;
                    }
                    let size = vertex_bytes(mesh);
                    if (limit > 0 && count >= limit)
                        || (budget > 0 && bytes > 0 && bytes + size > budget)
                    {
                        break;
                    }
                    let upload = pending.pop().expect("checked above");
                    renderer.upload(upload.key, &upload.mesh);
                    spent.push(upload);
                    count += 1;
                    bytes += size;
                }
                drop(upload_span);
                if let Some(flight) = raw_flight {
                    flight.note_upload_queue_work(work);
                    flight.note_mesh_uploads(count, bytes);
                }
                self.streaming.free(spent);
            }
            renderer.remove_columns(unloaded.iter().map(|c| (c.x, c.z)));
            // Meshes in the works are the view's terrain while it is new, then edits'.
            let meshing = self.streaming.busy() || !self.pending_uploads.is_empty();
            renderer.set_terrain_loading(arriving || (meshing && self.view_arrival.recent(now)));
        }
    }
}

/// Farthest first (the uploader pops from the back). Returns the number of
/// elements passed to a sort and to the merge, respectively.
fn order_uploads<T, K: Ord>(
    waiting: &mut Vec<T>,
    fresh: &mut Vec<T>,
    scratch: &mut Vec<T>,
    same_section: bool,
    priority: impl Fn(&T) -> K,
) -> [usize; 2] {
    if !same_section {
        waiting.append(fresh);
        if waiting.len() < 2 {
            return [0, 0];
        }
        waiting.sort_unstable_by_key(priority);
        return [waiting.len(), 0];
    }
    let sorted = if fresh.len() > 1 {
        fresh.sort_unstable_by_key(&priority);
        fresh.len()
    } else {
        0
    };
    if waiting
        .last()
        .zip(fresh.first())
        .is_none_or(|(old, new)| priority(old) <= priority(new))
    {
        waiting.append(fresh);
        return [sorted, 0];
    }

    // At equal distance, write the waiting mesh first so the fresh mesh is
    // popped first.
    let merged = waiting.len() + fresh.len();
    scratch.reserve(merged);
    let mut old = waiting.drain(..).peekable();
    let mut new = fresh.drain(..).peekable();
    while let (Some(a), Some(b)) = (old.peek(), new.peek()) {
        scratch.extend(if priority(a) <= priority(b) {
            old.next()
        } else {
            new.next()
        });
    }
    scratch.extend(old);
    scratch.extend(new);
    std::mem::swap(waiting, scratch);
    [sorted, merged]
}

/// Squared section-grid distance in `i128` so extreme camera coordinates cannot overflow.
fn section_distance_sq(key: (i32, i32, i32), centre: (i32, i32), cam_y: i32) -> i128 {
    let dx = i128::from(key.0) - i128::from(centre.0);
    let dy = i128::from(key.1) - i128::from(cam_y);
    let dz = i128::from(key.2) - i128::from(centre.1);
    dx * dx + dy * dy + dz * dz
}

#[cfg(test)]
mod tests {
    use super::{order_uploads, section_distance_sq};
    use std::cmp::Reverse;

    #[test]
    fn section_distance_sq_handles_extreme_coordinates_without_overflow() {
        let far = section_distance_sq(
            (i32::MAX, i32::MAX, i32::MAX),
            (i32::MIN, i32::MIN),
            i32::MIN,
        );
        let near = section_distance_sq(
            (i32::MIN, i32::MIN, i32::MIN),
            (i32::MIN, i32::MIN),
            i32::MIN,
        );
        assert_eq!(near, 0);
        assert!(far > near);
    }

    #[test]
    fn stationary_upload_budget_gives_fresh_meshes_priority_on_ties() {
        let mut waiting = vec![(20, "far"), (4, "waiting")];
        let mut fresh = vec![(1, "nearest"), (9, "middle"), (4, "fresh")];
        let mut scratch = Vec::new();
        let priority = |&(distance, _): &(i32, &str)| Reverse(distance);
        order_uploads(&mut waiting, &mut fresh, &mut scratch, true, priority);
        // Pin the stationary queue's fresh-first tie policy: two uploads leave
        // the waiting mesh first in line next frame. The old sort_unstable_by_key
        // did not guarantee this tie order.
        let selected: Vec<_> = std::iter::from_fn(|| waiting.pop().map(|(_, name)| name))
            .take(2)
            .collect();
        assert_eq!(selected, ["nearest", "fresh"]);
        assert_eq!(waiting.pop(), Some((4, "waiting")));
        assert_eq!(waiting.pop(), Some((9, "middle")));
        assert_eq!(waiting.pop(), Some((20, "far")));
    }
}
