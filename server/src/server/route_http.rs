use anyhow::Result;
use axum::{
    body::Body,
    http::{Method, Request, StatusCode},
    response::IntoResponse,
};
use serde_json::{json, Map, Value};
use std::sync::Arc;

use crate::{
    agents::{apply_created_session_identity, create_agent_session_params_for_project},
    auth::is_authorized_headers,
    constants::GXSERVER_PROTOCOL_VERSION,
    custom_session_tags::{
        clear_session_tags_missing_from_catalog, read_custom_session_tags,
        update_custom_session_tags,
    },
    domain::{read_optional_project_id, read_project_id, read_session_id, DomainStateError},
    extensions::{handle_extensions_http, serve_extension_static},
    navigation_history::{navigate_history, read_navigation_history, record_navigation_visit},
    notification_feed::{
        create_notification_endpoint, read_notification_feed_endpoint,
        update_notification_feed_endpoint, NOTIFICATION_FEED_CREATE_ENDPOINT,
        NOTIFICATION_FEED_READ_ENDPOINT, NOTIFICATION_FEED_UPDATE_ENDPOINT,
    },
    presentation::{
        increment_presentation_revision, list_previous_sessions, list_session_fork_branches,
        search_presentation_sessions,
    },
    protocol::{
        endpoint_for, is_remote_endpoint_allowed, protocol_mismatch_error, rpc_error, rpc_success,
        ListenerKind, MinimalHealthResponse, Transport,
    },
    remote_access::handle_remote_access_http,
    session_chat_files::{
        handle_read_session_chat_files_http, handle_read_session_chat_image_http,
        handle_save_session_chat_attachment_http, handle_save_session_chat_image_http,
    },
    session_chat_queue_runtime::handle_session_chat_queue_http,
    session_chat_read::handle_read_session_chat_http,
    session_chat_send::{
        handle_answer_session_chat_prompt_http, handle_handoff_session_chat_draft_http,
        handle_interrupt_session_chat_http, handle_replace_session_chat_draft_http,
        handle_send_session_chat_message_http,
    },
    session_chat_skills::handle_read_session_chat_skills_http,
    session_lifecycle,
    session_transcript_export::handle_export_session_transcript_http,
    session_transcript_size::handle_read_session_transcript_sizes_http,
    sidebar_hud::{
        create_sidebar_hud_settings_mutation, read_sidebar_hud, read_sidebar_hud_global_commands,
        GlobalSidebarCommandUpdate,
    },
    sidebar_project_collections::{
        assign_project_to_sidebar_collection, read_sidebar_project_collections,
        update_sidebar_project_collections,
    },
    sidebar_spaces::{read_sidebar_spaces, update_sidebar_spaces},
    tailcat::handle_tailcat_http,
    workspace_groups::{read_workspace_session_groups, update_workspace_session_groups},
};

use super::*;

pub(super) async fn route_http(
    state: Arc<AppState>,
    request: Request<Body>,
    request_id: String,
) -> RoutedResponse {
    let (parts, body) = request.into_parts();
    let path = parts.uri.path().to_string();
    let method = parts.method.clone();

    if method == Method::GET && path.starts_with("/ext/") {
        return serve_extension_static(state.extension_registry.clone(), path).await;
    }

    if method == Method::GET && path.starts_with("/project-view-report/") {
        return crate::project_views::serve(path).await;
    }

    // OpenDots: Composio's OAuth redirect lands in the user's browser, which has no gxserver
    // token; the handler trusts only the OAuth `state` its own sign-in issued.
    if method == Method::GET && path == crate::open_dots::composio::OAUTH_REDIRECT_PATH {
        let query = parts.uri.query().map(str::to_string);
        return crate::open_dots::composio::serve_oauth_redirect(state.clone(), query).await;
    }

    let endpoint = endpoint_for(&path);

    /*
    CDXC:ServerApi 2026-06-22-04:10:
    Rust routing must preserve TypeScript's protocol gate order: CORS/OPTIONS is answered before minimal health, auth, method, body, and protocol checks. Unknown or WebSocket-only OPTIONS requests therefore return the HTTP-endpoint 404 envelope instead of the generic endpoint lookup message.
    */
    if method == Method::OPTIONS {
        let Some(endpoint) = endpoint else {
            return routed_json(
                None,
                StatusCode::NOT_FOUND,
                rpc_error(
                    "notFound",
                    format!("{path} is not a gxserver HTTP endpoint."),
                    Some(request_id),
                ),
            );
        };
        if endpoint.transport != Transport::Http {
            return routed_json(
                Some(endpoint.path),
                StatusCode::NOT_FOUND,
                rpc_error(
                    "notFound",
                    format!("{path} is not a gxserver HTTP endpoint."),
                    Some(request_id),
                ),
            );
        }
        if !is_remote_endpoint_allowed(ListenerKind::Local, endpoint.permission) {
            return routed_json(
                Some(endpoint.path.clone()),
                StatusCode::FORBIDDEN,
                rpc_error(
                    "forbidden",
                    format!(
                        "{} is not available on the remote gxserver listener.",
                        endpoint.path
                    ),
                    Some(request_id),
                ),
            );
        }
        return RoutedResponse {
            endpoint_path: Some(endpoint.path),
            response: StatusCode::NO_CONTENT.into_response(),
        };
    }

    if method == Method::GET && path == "/api/health" {
        return routed_json(
            Some("/api/health".to_string()),
            StatusCode::OK,
            MinimalHealthResponse::new(&state.version),
        );
    }

    let Some(endpoint) = endpoint else {
        return routed_json(
            None,
            StatusCode::NOT_FOUND,
            rpc_error(
                "notFound",
                format!("No gxserver endpoint for {} {}.", method.as_str(), path),
                Some(request_id),
            ),
        );
    };

    if endpoint.transport != Transport::Http {
        return routed_json(
            None,
            StatusCode::NOT_FOUND,
            rpc_error(
                "notFound",
                format!("No gxserver endpoint for {} {}.", method.as_str(), path),
                Some(request_id),
            ),
        );
    }

    if endpoint.path != "/api/health/server" && method != Method::POST {
        return routed_json(
            Some(endpoint.path.clone()),
            StatusCode::METHOD_NOT_ALLOWED,
            rpc_error(
                "methodNotAllowed",
                format!("{} requires POST.", endpoint.path),
                Some(request_id),
            ),
        );
    }
    if endpoint.path == "/api/health/server" && method != Method::GET {
        return routed_json(
            Some(endpoint.path.clone()),
            StatusCode::METHOD_NOT_ALLOWED,
            rpc_error(
                "methodNotAllowed",
                format!("{} requires GET.", endpoint.path),
                Some(request_id),
            ),
        );
    }

    let token_extension_id = state
        .extension_registry
        .authorize_api_token(&parts.headers, &endpoint.path);
    if endpoint.requires_auth
        && !is_authorized_headers(&parts.headers, &state.auth_token)
        && token_extension_id.is_none()
    {
        return routed_json(
            Some(endpoint.path),
            StatusCode::UNAUTHORIZED,
            rpc_error(
                "unauthorized",
                "gxserver auth token is required for this endpoint.",
                Some(request_id),
            ),
        );
    }

    let body_json = if method == Method::POST {
        let body_limit_bytes = json_body_limit_bytes(&endpoint.path);
        match read_json_body(&parts.headers, body, body_limit_bytes).await {
            Ok(value) => value,
            Err(ReadBodyError::TooLarge) => {
                return routed_json(
                    Some(endpoint.path),
                    StatusCode::PAYLOAD_TOO_LARGE,
                    rpc_error(
                        "badRequest",
                        format!(
                            "Request body exceeds the gxserver JSON RPC limit of {body_limit_bytes} bytes."
                        ),
                        Some(request_id),
                    ),
                );
            }
            Err(ReadBodyError::InvalidJson) => {
                return routed_json(
                    Some(endpoint.path),
                    StatusCode::BAD_REQUEST,
                    rpc_error(
                        "badRequest",
                        "Request body must be valid JSON.",
                        Some(request_id),
                    ),
                );
            }
        }
    } else {
        json!({})
    };

    if endpoint.requires_protocol_version && token_extension_id.is_none() {
        let protocol_version = read_protocol_version(&parts.headers, &parts.uri, Some(&body_json));
        if !is_expected_protocol_version(protocol_version.as_ref()) {
            return routed_json(
                Some(endpoint.path),
                StatusCode::UPGRADE_REQUIRED,
                protocol_mismatch_error(protocol_version, Some(request_id)),
            );
        }
    }

    if !is_remote_endpoint_allowed(ListenerKind::Local, endpoint.permission) {
        return routed_json(
            Some(endpoint.path.clone()),
            StatusCode::FORBIDDEN,
            rpc_error(
                "forbidden",
                format!(
                    "{} is not available on the remote gxserver listener.",
                    endpoint.path
                ),
                Some(request_id),
            ),
        );
    }

    match endpoint.path.as_str() {
        "/api/health/server" => routed_json(
            Some(endpoint.path),
            StatusCode::OK,
            create_authenticated_health(&state),
        ),
        "/api/createProject" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project = repository.create_project(params)?;
                let project_id = value_text(&project, "projectId")?;
                schedule_presentation_project_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    "projectAdded",
                )?;
                Ok(json!({ "project": project }))
            },
        ),
        "/api/updateProject" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project = repository.update_project(params)?;
                let project_id = value_text(&project, "projectId")?;
                schedule_presentation_project_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    "projectUpdated",
                )?;
                Ok(json!({ "project": project }))
            },
        ),
        "/api/relocateProject" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project = repository.relocate_project(params)?;
                let project_id = value_text(&project, "projectId")?;
                schedule_presentation_project_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    "projectUpdated",
                )?;
                Ok(json!({ "project": project }))
            },
        ),
        "/api/listProjects" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, _, _| {
                repository
                    .list_projects()
                    .map(|projects| json!({ "projects": projects }))
            },
        ),
        "/api/listRecentProjects" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, _, _| {
                repository
                    .list_recent_projects()
                    .map(|recent_projects| json!({ "recentProjects": recent_projects }))
            },
        ),
        "/api/closeProjectToRecent" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                /*
                CDXC:Projects 2026-06-24-12:38:
                GPUI's reused SidebarApp sends the same close-vs-remove command split as macOS. Close parks the canonical gxserver project with a server timestamp and broadcasts a presentation removal for active groups; remove remains the hard-delete endpoint.
                */
                let project_id = read_project_id(params)?;
                let project = repository.close_project_to_recent(&project_id)?;
                schedule_presentation_project_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    "projectUpdated",
                )?;
                let recent_projects = repository.list_recent_projects()?;
                Ok(json!({ "project": project, "recentProjects": recent_projects }))
            },
        ),
        "/api/restoreRecentProject" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                /*
                CDXC:Projects 2026-06-24-12:27:
                GPUI restores Recent Projects through gxserver project ids.
                The daemon clears explicit parked state and publishes the
                project presentation update; clients must not reconstruct rows
                from labels, stopped sessions, trusted client paths, or command
                output.
                */
                let project_id = read_project_id(params)?;
                let project = repository.restore_recent_project(&project_id)?;
                schedule_presentation_project_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    "projectUpdated",
                )?;
                let recent_projects = repository.list_recent_projects()?;
                Ok(json!({ "project": project, "recentProjects": recent_projects }))
            },
        ),
        "/api/removeRecentProject" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project_id = read_project_id(params)?;
                let project = repository.remove_recent_project(&project_id)?;
                schedule_presentation_project_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    "projectRemoved",
                )?;
                let recent_projects = repository.list_recent_projects()?;
                Ok(json!({ "project": project, "recentProjects": recent_projects }))
            },
        ),
        "/api/readProjectStatus" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project_id = read_project_id(params)?;
                let project = repository.get_project(&project_id)?.ok_or_else(|| {
                    DomainStateError::not_found(format!("Project {project_id} does not exist."))
                })?;
                /*
                CDXC:Projects 2026-06-22-06:21:
                readProjectStatus is a polling project-status read, but TypeScript gxserver repairs live zmx process identity before returning the project/session graph and schedules agent metadata title checks for eligible sessions. Keep those side effects here instead of treating the endpoint as a plain repository read so Rust clients receive the same status projection.
                */
                let sessions = repository.list_sessions(Some(project_id.as_str()))?;
                let mut sessions_changed =
                    sync_zmx_provider_existence(&state, db, repository, &sessions)?;
                sessions_changed |= sync_live_zmx_process_identities(
                    &state,
                    db,
                    repository,
                    &sessions,
                    None,
                    "read-project-status",
                )?;
                let sessions = if sessions_changed {
                    repository.list_sessions(Some(project_id.as_str()))?
                } else {
                    sessions
                };
                schedule_agent_title_metadata_checks_for_sessions(&state, &sessions);
                Ok(json!({
                    "project": project,
                    "sessions": sessions,
                }))
            },
        ),
        "/api/addProjectPath" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project = repository.add_project_path(params)?;
                let project_id = value_text(&project, "projectId")?;
                schedule_presentation_project_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    "projectAdded",
                )?;
                Ok(json!({ "project": project }))
            },
        ),
        "/api/recordClientEvent" => {
            handle_record_client_event_http(endpoint.path, request_id, &body_json)
        }
        "/api/createQuickProject" => {
            let home_dir = state.paths.home_dir.clone();
            let quick_project_state = state.clone();
            handle_domain_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
                move |repository, db, params, _| {
                    let project_params = create_quick_project_params(&home_dir, params)?;
                    let project = repository.add_project_path(&project_params)?;
                    let project_id = value_text(&project, "projectId")?;
                    schedule_presentation_project_delta(
                        &quick_project_state,
                        db,
                        repository,
                        &project_id,
                        "projectAdded",
                    )?;
                    Ok(json!({ "project": project }))
                },
            )
        }
        // Off the async runtime like the startup pass: the sync reads the profiles folder and
        // writes the database.
        "/api/syncBotProjects" => {
            let worker_state = state.clone();
            let worker_endpoint = endpoint.path.clone();
            let worker_request_id = request_id.clone();
            match tokio::task::spawn_blocking(move || {
                handle_domain_http(
                    &worker_state,
                    worker_endpoint,
                    worker_request_id,
                    &body_json,
                    |repository, db, _, _| {
                        bot_sync::sync_and_publish_bot_projects(&worker_state, db, repository)
                    },
                )
            })
            .await
            {
                Ok(response) => response,
                Err(error) => domain_error_response(
                    endpoint.path,
                    request_id,
                    DomainStateError::corrupt_state(format!("Bot project sync failed: {error}")),
                ),
            }
        }
        "/api/listBotFeed" => {
            let feed_state = state.clone();
            match tokio::task::spawn_blocking(move || {
                crate::bot_feed::read_bot_feed(
                    &feed_state.paths,
                    &crate::session_chat_hermes::hermes_home(),
                )
            })
            .await
            {
                Ok(feed) => routed_json(
                    Some(endpoint.path),
                    StatusCode::OK,
                    rpc_success(request_id, feed),
                ),
                Err(error) => domain_error_response(
                    endpoint.path,
                    request_id,
                    DomainStateError::corrupt_state(format!("Bot feed read failed: {error}")),
                ),
            }
        }
        "/api/openDotsRpc" => {
            crate::open_dots::handle_open_dots_rpc_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
            )
            .await
        }
        "/api/openDotsTool" => {
            crate::open_dots::handle_open_dots_tool_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
            )
            .await
        }
        "/api/listProjectWorktrees"
        | "/api/createProjectWorktree"
        | "/api/openProjectWorktree"
        | "/api/mergeWorktreeIntoMain"
        | "/api/checkoutProjectNewBranch" => {
            handle_project_worktree_operation_http(&state, endpoint.path, request_id, &body_json)
                .await
        }
        "/api/removeProject" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project_id = read_project_id(params)?;
                let project = repository.remove_project(&project_id)?;
                schedule_presentation_project_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    "projectUpdated",
                )?;
                Ok(json!({ "project": project }))
            },
        ),
        "/api/deleteWorktreeProject" => {
            handle_delete_worktree_project_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/renameWorktreeProject" => {
            handle_rename_worktree_project_http(&state, endpoint.path, request_id, &body_json).await
        }
        /*
        CDXC:Worktrees 2026-07-29-00:00:
        Sidebar V2's worktree flow. Unlike `createProjectWorktree`, these
        endpoints never register the worktree as a project: the checkout is an
        attribute of one session (its cwd), and the branch shown on the card
        comes from the per-session git probe reading that cwd.
        */
        "/api/createWorktreeSession" | "/api/removeSessionWorktree" => {
            handle_worktree_session_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/createSession" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                // Only gxserver makes a box session (a box create, or reopening one from history).
                let params = &crate::agentbox::without_client_agentbox_record(params);
                let created_session = repository.create_session(params, false)?;
                let session = apply_created_session_identity(repository, &created_session, params)?;
                let project_id = value_text(&session, "projectId")?;
                let session_id = value_text(&session, "sessionId")?;
                restore_parked_project_for_new_session(&state, db, repository, &project_id)?;
                schedule_presentation_session_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    &session_id,
                )?;
                Ok(json!({ "session": session }))
            },
        ),
        "/api/createAgentSession" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project = repository.resolve_create_session_project(params)?;
                let params =
                    &coordinator_runtime::prepare_coordinator_create_params(&state, params)?;
                let create_params = create_agent_session_params_for_project(db, &project, params)?;
                let created_session = repository.create_session(&create_params, false)?;
                let session =
                    apply_created_session_identity(repository, &created_session, &create_params)?;
                crate::coordinators::register_created_coordinator(db, params, &session)?;
                let project_id = value_text(&session, "projectId")?;
                let session_id = value_text(&session, "sessionId")?;
                restore_parked_project_for_new_session(&state, db, repository, &project_id)?;
                schedule_presentation_session_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    &session_id,
                )?;
                Ok(json!({ "session": session }))
            },
        ),
        "/api/readResourceSessionOwners" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, params, _| {
                let names = params
                    .get("zmxNames")
                    .and_then(Value::as_array)
                    .filter(|names| names.len() <= 4096)
                    .ok_or_else(|| {
                        DomainStateError::bad_request("zmxNames must be a bounded array.")
                    })?;
                let names = names
                    .iter()
                    .map(|name| {
                        name.as_str()
                            .filter(|name| !name.is_empty() && name.len() <= 256)
                            .map(str::to_string)
                            .ok_or_else(|| {
                                DomainStateError::bad_request("Invalid zmx session name.")
                            })
                    })
                    .collect::<Result<std::collections::HashSet<_>, _>>()?;
                Ok(json!({ "sessions": repository.resource_session_owners(&names)? }))
            },
        ),
        "/api/listSessions" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project_id = read_optional_project_id(params)?;
                /*
                CDXC:StateSync 2026-09-01:
                A registry accumulates stopped agent history forever — thousands
                of rows on a working machine — and every one of them was
                hydrated, serialized, and shipped on each poll even though the
                CLI, the mobile inventory, and the desktop close check all
                discard stopped rows on arrival. `includeStopped` lets those
                callers say so up front; it defaults to true, so the endpoint's
                published contract is unchanged for anyone who does not send it.

                The filter is the durable `lifecycleState <> 'stopped'`, which
                is exactly the predicate those callers apply client-side, so an
                opted-in caller sees the same rows it would have kept anyway.

                The three sync passes below only ever act on rows whose stored
                `lifecycleState` is `running` (see `session_state_sync.rs`), and
                every such row survives the filter, so they operate on the same
                candidate set either way. The one pass that also touches other
                rows is the working-directory title repair inside
                `sync_session_state_sidecars`; it stays exhaustive on the one
                unfiltered path left (`/api/readProjectStatus`) and is
                idempotent, so a narrower list simply defers it. Since
                2026-09-11 the snapshot poll and the presentation subscribe use
                `list_presentation_sessions`, which also covers every row the
                repair could change while it is still visible.
                */
                let include_stopped = params
                    .get("includeStopped")
                    .and_then(Value::as_bool)
                    .unwrap_or(true);
                let list_sessions = |project_id: Option<&str>| {
                    if include_stopped {
                        repository.list_sessions(project_id)
                    } else {
                        repository.list_sessions_excluding_stopped(project_id)
                    }
                };
                /*
                CDXC:StateSync 2026-09-01:
                One `list_sessions` feeds all three sync passes and the
                response. The passes can mutate rows, so re-read only when one
                of them reports an actual change.
                */
                let sessions = list_sessions(project_id.as_deref())?;
                let mut sessions_changed = sync_session_state_sidecars(
                    &state,
                    db,
                    repository,
                    &sessions,
                    "list-sessions",
                )?;
                sessions_changed |= sync_zmx_provider_existence(&state, db, repository, &sessions)?;
                sessions_changed |= sync_live_zmx_process_identities(
                    &state,
                    db,
                    repository,
                    &sessions,
                    None,
                    "list-sessions",
                )?;
                let sessions = if sessions_changed {
                    list_sessions(project_id.as_deref())?
                } else {
                    sessions
                };
                Ok(json!({ "sessions": sessions }))
            },
        ),
        "/api/updateSession" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let session = repository.update_session(params)?;
                let project_id = value_text(&session, "projectId")?;
                let session_id = value_text(&session, "sessionId")?;
                schedule_presentation_session_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    &session_id,
                )?;
                Ok(json!({ "session": session }))
            },
        ),
        "/api/updateSessionOrder" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let sessions = repository.update_session_order(params)?;
                for session in &sessions {
                    let project_id = value_text(session, "projectId")?;
                    let session_id = value_text(session, "sessionId")?;
                    schedule_presentation_session_delta(
                        &state,
                        db,
                        repository,
                        &project_id,
                        &session_id,
                    )?;
                }
                Ok(json!({ "sessions": sessions }))
            },
        ),
        /*
        CDXC:StateSync 2026-07-29-00:00:
        Sidebar V2's settle/snooze commands. Guards live in
        `session_lifecycle` so a stale or raced client cannot park working or
        blocked-on-you work behind a settle, and every real change emits a
        presentation delta so all clients reclassify live. A no-op (double
        click, bulk settle over an already-settled row) intentionally skips the
        delta instead of churning the presentation revision.
        */
        "/api/settleSession"
        | "/api/unsettleSession"
        | "/api/snoozeSession"
        | "/api/unsnoozeSession" => {
            let lifecycle_path = endpoint.path.clone();
            let lifecycle_state = state.clone();
            handle_domain_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
                move |repository, db, params, _| {
                    let project_id = read_project_id(params)?;
                    let session_id = read_session_id(params)?;
                    let now = now_iso();
                    let outcome = match lifecycle_path.as_str() {
                        "/api/settleSession" => session_lifecycle::settle_session(
                            repository,
                            &project_id,
                            &session_id,
                            &now,
                        )?,
                        "/api/unsettleSession" => session_lifecycle::unsettle_session(
                            repository,
                            &project_id,
                            &session_id,
                            &now,
                        )?,
                        "/api/snoozeSession" => {
                            let snoozed_until = params
                                .get("snoozedUntil")
                                .and_then(Value::as_str)
                                .map(str::trim)
                                .filter(|value| !value.is_empty())
                                .ok_or_else(|| {
                                    DomainStateError::bad_request(
                                        "snoozedUntil must be an ISO timestamp in the future.",
                                    )
                                })?;
                            session_lifecycle::snooze_session(
                                repository,
                                &project_id,
                                &session_id,
                                snoozed_until,
                                &now,
                            )?
                        }
                        _ => session_lifecycle::unsnooze_session(
                            repository,
                            &project_id,
                            &session_id,
                            &now,
                        )?,
                    };
                    if outcome.changed {
                        schedule_presentation_session_delta(
                            &lifecycle_state,
                            db,
                            repository,
                            &project_id,
                            &session_id,
                        )?;
                    }
                    Ok(json!({
                        "changed": outcome.changed,
                        "session": outcome.session,
                    }))
                },
            )
        }
        "/api/removeSession" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let session = repository.remove_session(params)?;
                crate::agentbox::stop_session_box_in_background(repository, &session);
                /*
                CDXC:Drafts 2026-08-28:
                Removing a DRAFT also kills its background agent CLI. The row
                being deleted is the last thing that pointed at that zmx daemon,
                so deleting a draft would otherwise leave an orphaned CLI running
                with nothing in any sidebar able to stop it. Scoped strictly to
                drafts: removing any other session behaves exactly as it did
                before — those are sessions the user is expected to have closed
                deliberately, and changing that is not this feature's call to
                make.
                */
                let removed_a_draft = crate::agents::session_is_draft(&session);
                crate::agents::kill_draft_session_provider(&session);
                let project_id = value_text(&session, "projectId")?;
                let session_id = value_text(&session, "sessionId")?;
                schedule_presentation_session_delta(
                    &state,
                    db,
                    repository,
                    &project_id,
                    &session_id,
                )?;
                if removed_a_draft {
                    /*
                    CDXC:Drafts 2026-08-28:
                    A quick chat's draft was created inside a throwaway
                    `~/ghostex/chats` workspace made for it alone, so deleting
                    the draft has to collect that workspace too or the sidebar
                    accumulates empty "Chat …" projects over real directories
                    nobody will ever open. A no-op unless the project is a quick
                    one with no sessions left; see
                    `discard_stranded_quick_project` for the guards on its
                    directory delete.
                    */
                    if crate::agents::discard_stranded_quick_project(
                        repository,
                        &state.paths.home_dir,
                        &project_id,
                    )? {
                        schedule_presentation_project_delta(
                            &state,
                            db,
                            repository,
                            &project_id,
                            "projectUpdated",
                        )?;
                    }
                }
                Ok(json!({ "session": session }))
            },
        ),
        "/api/readPresentationSnapshot" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, _, server_id| {
                /*
                CDXC:StateSync 2026-09-01:
                One session list feeds all three sync passes and the snapshot
                projection. The passes can mutate rows, so re-read only when
                one of them reports an actual change.

                CDXC:StateSync 2026-09-11 WHY:
                The list is presentation-scoped: the sync passes only act on
                `running` rows, and the projection discards every stopped row
                that is not pinned, parked, favorite, or tagged, so hydrating
                the thousands of other stopped rows on every two-second poll
                was pure cost. Fork families are derived inside the snapshot
                from the narrow fork-row read over the whole registry.
                */
                let sessions = repository.list_presentation_sessions()?;
                /*
                CDXC:StateSync 2026-09-18 WHY:
                Every client read ran the three repair passes, and the desktop
                alone reads this about once a second, so the zmx process scan
                behind sync_live_zmx_process_identities ran continuously (13%
                of gxserver's CPU in a sample). The passes repair durable rows,
                so a read within a few seconds of the last pass sees the same
                rows it would have repaired; run them at most every 5 seconds.
                */
                let mut sessions_changed = false;
                if presentation_snapshot_sync_due() {
                    sessions_changed = sync_session_state_sidecars(
                        &state,
                        db,
                        repository,
                        &sessions,
                        "read-presentation-snapshot",
                    )?;
                    sessions_changed |=
                        sync_zmx_provider_existence(&state, db, repository, &sessions)?;
                    sessions_changed |= sync_live_zmx_process_identities(
                        &state,
                        db,
                        repository,
                        &sessions,
                        None,
                        "read-presentation-snapshot",
                    )?;
                }
                let sessions = if sessions_changed {
                    repository.list_presentation_sessions()?
                } else {
                    sessions
                };
                read_presentation_snapshot_in_sequence(&state, db, server_id, sessions)
                    .map(|snapshot| json!({ "snapshot": snapshot }))
            },
        ),
        "/api/readSidebarHud" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, params, _| {
                /*
                CDXC:AgentLauncher 2026-06-24-20:34:
                GPUI Settings and SidebarApp read normalized launcher/action HUD rows through gxserver so app-modal Rust does not hand-mirror the shared TypeScript projection. The response is derived only from project domain metadata and carries no paths, project names, prompts, tokens, stdout/stderr, daemon bodies, or renderer payload authority.
                */
                let projects = repository.list_projects()?;
                let active_project_id = params
                    .get("activeProjectId")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty());
                let mut hud = read_sidebar_hud(&projects, active_project_id);
                /*
                CDXC:AgentLauncher 2026-07-12-00:00:
                React Native Android renders agent-launcher and quick-action buttons for
                every visible project at once, so the mobile CLI transport asks
                for per-project command rows in one round trip instead of one
                readSidebarHud call per project each poll.
                */
                apply_commands_by_project_if_requested(&mut hud, &projects, params);
                /*
                CDXC:AgentLauncher 2026-08-01-16:00:
                Global Actions live in their own daemon table rather than in
                project metadata, so they are attached here instead of inside
                read_sidebar_hud, which stays a pure projection of project rows.
                Served unconditionally rather than behind an opt-in flag: the
                list is one small array with no per-project fan-out, and every
                surface that renders the tab strip needs it on first paint.
                */
                if let Some(hud) = hud.as_object_mut() {
                    hud.insert(
                        "globalCommands".to_string(),
                        read_sidebar_hud_global_commands(
                            &repository.list_global_sidebar_commands()?,
                        ),
                    );
                }
                Ok(hud)
            },
        ),
        "/api/mutateSidebarHudSettings" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                /*
                CDXC:AgentLauncher 2026-06-24-20:54:
                Settings mutation RPCs write through the production project repository after gxserver normalizes the narrow agent/action intent. Return refreshed HUD rows and updated project rows so GPUI clients do not reparse raw metadata or log command text, URLs, project names, paths, prompts, tokens, stdout/stderr, daemon bodies, or renderer payload contents.
                */
                let projects = repository.list_projects()?;
                let mutation = create_sidebar_hud_settings_mutation(&projects, params)?;
                let hud_active_project_id = mutation.hud_active_project_id;
                let mut item_ids = mutation.item_ids;
                /*
                CDXC:AgentLauncher 2026-08-01-16:00:
                A reorder response must echo the order the daemon actually
                stored — the sidebar treats itemIds as the confirmation for its
                optimistic reorder and falls back to an empty list without it.
                The stored order can differ from the ids the client sent, since
                the repository keeps unlisted actions instead of dropping them.
                */
                let global_command_order_requested = matches!(
                    mutation.global_command_update,
                    Some(GlobalSidebarCommandUpdate::Order { .. })
                );
                let global_command_written = mutation.global_command_update.is_some();
                let mut updated_projects = Vec::new();
                for update in mutation.updates {
                    let project = repository.update_project(&update.params)?;
                    schedule_presentation_project_delta(
                        &state,
                        db,
                        repository,
                        &update.project_id,
                        "projectUpdated",
                    )?;
                    updated_projects.push(project);
                }
                /*
                CDXC:AgentLauncher 2026-08-01-16:00:
                A Global Action write touches no project row, so it schedules no
                projectUpdated presentation delta.

                CDXC:AgentLauncher 2026-08-07:
                It announces itself with its own event instead. Only the caller
                sees the refreshed HUD this response carries; every other live
                surface learns about HUD changes from a broadcast, and none of
                them polls the HUD on a timer. The GPUI sidebar refetches it
                when a projectUpdated delta arrives, which is why a project
                Action edit reaches the row at once and a Global Action edit did
                not — the row kept the stale list until some unrelated project
                delta happened to fire. The event carries no list, because
                /api/readSidebarHud stays the single projection of it, and it
                bumps the presentation revision so snapshot pollers converge as
                well, exactly like the sidebar-collection writes below.
                */
                match mutation.global_command_update {
                    Some(GlobalSidebarCommandUpdate::Save {
                        command_id,
                        definition,
                    }) => repository.save_global_sidebar_command(&command_id, &definition)?,
                    Some(GlobalSidebarCommandUpdate::Delete { command_id }) => {
                        repository.delete_global_sidebar_command(&command_id)?
                    }
                    Some(GlobalSidebarCommandUpdate::Order { command_ids }) => {
                        repository.order_global_sidebar_commands(&command_ids)?
                    }
                    None => {}
                }
                if global_command_written {
                    let _event_sequence = lock_presentation_event_sequence(&state)?;
                    let revision = increment_presentation_revision(db)?;
                    state.event_hub.broadcast(json!({
                        "protocolVersion": GXSERVER_PROTOCOL_VERSION,
                        "revision": revision,
                        "serverId": state.metadata.server_id.clone(),
                        "type": "globalSidebarCommandsChanged",
                    }));
                }
                let projects = repository.list_projects()?;
                let mut hud = read_sidebar_hud(&projects, hud_active_project_id.as_deref());
                let global_commands =
                    read_sidebar_hud_global_commands(&repository.list_global_sidebar_commands()?);
                if global_command_order_requested {
                    item_ids = Some(
                        global_commands
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter_map(Value::as_object)
                            .filter_map(|command| command.get("commandId"))
                            .filter_map(Value::as_str)
                            .map(str::to_string)
                            .collect(),
                    );
                }
                if let Some(hud) = hud.as_object_mut() {
                    hud.insert("globalCommands".to_string(), global_commands);
                }
                /*
                CDXC:Projects 2026-08-01:
                Clients that render per-project quick actions (GPUI sidebar rows)
                replace their whole HUD snapshot with this response, so the
                mutation mirrors readSidebarHud's opt-in commandsByProject block.
                Without it a Settings save would drop the per-project rows until
                the next full HUD poll.
                */
                apply_commands_by_project_if_requested(&mut hud, &projects, params);
                let mut result = Map::new();
                result.insert("hud".to_string(), hud);
                if let Some(item_ids) = item_ids {
                    result.insert(
                        "itemIds".to_string(),
                        Value::Array(item_ids.into_iter().map(Value::String).collect()),
                    );
                }
                result.insert("projects".to_string(), Value::Array(updated_projects));
                Ok(Value::Object(result))
            },
        ),
        /*
        CDXC:Navigation 2026-08-19:
        Titlebar Back/Forward is one daemon-owned trail of previously active
        sessions and projects, shared by the gpui desktop titlebar and the web
        titlebar — see `navigation_history`. These three calls carry only
        opaque routing ids plus the display titles the sidebar already renders,
        so they sit with the other sidebar-state endpoints and need no
        repository or database access.
        */
        "/api/readNavigationHistory" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, _, params, _| read_navigation_history(params),
        ),
        "/api/recordNavigationVisit" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, _, params, _| record_navigation_visit(params),
        ),
        "/api/navigateHistory" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, _, params, _| navigate_history(params),
        ),
        NOTIFICATION_FEED_READ_ENDPOINT => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, _, _| read_notification_feed_endpoint(db),
        ),
        NOTIFICATION_FEED_UPDATE_ENDPOINT => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, params, _| update_notification_feed_endpoint(&state, db, params),
        ),
        NOTIFICATION_FEED_CREATE_ENDPOINT => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                create_notification_endpoint(&state, repository, db, params)
            },
        ),
        "/api/readWorkspaceSessionGroups" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, _, _| {
                read_workspace_session_groups(db).map(|groups| json!({ "groups": groups }))
            },
        ),
        "/api/updateWorkspaceSessionGroups" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, params, _| {
                /*
                CDXC:Sessions 2026-07-12-00:00:
                GPUI write-through-syncs its whole normalized named-group overlay
                after each local edit. Bump the presentation revision and broadcast
                a dedicated event so snapshot pollers (mobile via CLI) and live
                sidebar clients converge without re-sending session rows.
                */
                let _event_sequence = lock_presentation_event_sequence(&state)?;
                let groups = update_workspace_session_groups(db, params)?;
                let revision = increment_presentation_revision(db)?;
                state.event_hub.broadcast(json!({
                    "groups": groups.clone(),
                    "protocolVersion": GXSERVER_PROTOCOL_VERSION,
                    "revision": revision,
                    "serverId": state.metadata.server_id.clone(),
                    "type": "workspaceGroupsChanged",
                }));
                Ok(json!({ "groups": groups }))
            },
        ),
        "/api/readSidebarProjectCollections" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, _, _| {
                read_sidebar_project_collections(db)
                    .map(|collections| json!({ "sidebarProjectCollections": collections }))
            },
        ),
        "/api/updateSidebarProjectCollections" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, params, _| {
                /*
                CDXC:Projects 2026-07-18-00:00:
                Editors write-through-sync the whole normalized project-collection
                overlay after each local edit. Bump the presentation revision and
                broadcast a dedicated event so snapshot pollers (mobile via CLI)
                and live sidebar clients converge without re-sending project rows.
                */
                let _event_sequence = lock_presentation_event_sequence(&state)?;
                let previous_collections = read_sidebar_project_collections(db)?;
                let collections = update_sidebar_project_collections(db, params)?;
                let revision = increment_presentation_revision(db)?;
                state.event_hub.broadcast(json!({
                    "protocolVersion": GXSERVER_PROTOCOL_VERSION,
                    "revision": revision,
                    "serverId": state.metadata.server_id.clone(),
                    "sidebarProjectCollections": collections.clone(),
                    "type": "sidebarProjectCollectionsChanged",
                }));
                broadcast_pruned_sidebar_spaces(&state, db, &previous_collections, &collections)?;
                Ok(json!({ "sidebarProjectCollections": collections }))
            },
        ),
        "/api/assignProjectToSidebarCollection" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let project_id = resolve_sidebar_collection_project_id(repository, params)?;
                let collection_title = params
                    .get("collectionTitle")
                    .or_else(|| params.get("group"))
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|title| !title.is_empty())
                    .ok_or_else(|| {
                        DomainStateError::bad_request(
                            "group-project requires a non-empty sidebar group title.",
                        )
                    })?;
                let _event_sequence = lock_presentation_event_sequence(&state)?;
                let previous_collections = read_sidebar_project_collections(db)?;
                let collections =
                    assign_project_to_sidebar_collection(db, &project_id, collection_title)?;
                let revision = increment_presentation_revision(db)?;
                state.event_hub.broadcast(json!({
                    "protocolVersion": GXSERVER_PROTOCOL_VERSION,
                    "revision": revision,
                    "serverId": state.metadata.server_id.clone(),
                    "sidebarProjectCollections": collections.clone(),
                    "type": "sidebarProjectCollectionsChanged",
                }));
                broadcast_pruned_sidebar_spaces(&state, db, &previous_collections, &collections)?;
                Ok(json!({
                    "projectId": project_id,
                    "sidebarProjectCollections": collections,
                }))
            },
        ),
        "/api/readSidebarSpaces" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, _, _| read_sidebar_spaces(db).map(|spaces| json!({ "sidebarSpaces": spaces })),
        ),
        "/api/updateSidebarSpaces" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, params, _| {
                /*
                CDXC:Spaces 2026-08-27:
                Space editors write-through-sync the whole normalized Space
                document after each local edit, exactly like the project
                collections beside them. Bump the presentation revision and
                broadcast a dedicated event so snapshot pollers (mobile via CLI)
                and live sidebar clients converge without re-sending project rows.
                */
                let _event_sequence = lock_presentation_event_sequence(&state)?;
                let spaces = update_sidebar_spaces(db, params)?;
                let revision = increment_presentation_revision(db)?;
                state.event_hub.broadcast(json!({
                    "protocolVersion": GXSERVER_PROTOCOL_VERSION,
                    "revision": revision,
                    "serverId": state.metadata.server_id.clone(),
                    "sidebarSpaces": spaces.clone(),
                    "type": "sidebarSpacesChanged",
                }));
                Ok(json!({ "sidebarSpaces": spaces }))
            },
        ),
        "/api/readCustomSessionTags" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, _, _| {
                read_custom_session_tags(db).map(|tags| json!({ "customSessionTags": tags }))
            },
        ),
        "/api/updateCustomSessionTags" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                /*
                CDXC:Sessions 2026-09-11 WHY:
                Tag editors write-through-sync the whole normalized catalog
                after each local edit, exactly like Spaces. A tag deleted from
                the catalog is cleared from every session that carried it
                inside the same sequenced write, so no client ever sees a
                session pointing at an id the daemon no longer knows; each
                cleared session then gets its ordinary presentation delta so
                sidebars drop the marker without a full snapshot reload.
                */
                let (tags, cleared) = {
                    let _event_sequence = lock_presentation_event_sequence(&state)?;
                    let tags = update_custom_session_tags(db, params)?;
                    let cleared = clear_session_tags_missing_from_catalog(db, &tags)?;
                    let revision = increment_presentation_revision(db)?;
                    state.event_hub.broadcast(json!({
                        "protocolVersion": GXSERVER_PROTOCOL_VERSION,
                        "revision": revision,
                        "serverId": state.metadata.server_id.clone(),
                        "customSessionTags": tags.clone(),
                        "type": "customSessionTagsChanged",
                    }));
                    (tags, cleared)
                };
                for (project_id, session_id) in &cleared {
                    schedule_presentation_session_delta(
                        &state, db, repository, project_id, session_id,
                    )?;
                }
                Ok(json!({ "customSessionTags": tags }))
            },
        ),
        "/api/readAppUserData" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, _, _| repository.read_app_user_data(),
        ),
        "/api/readAutomationState"
        | "/api/saveAutomation"
        | "/api/deleteAutomation"
        | "/api/runAutomationNow"
        | "/api/setAutomationEnabled"
        | "/api/archiveAutomationRun"
        | "/api/markAutomationRunRead" => {
            handle_automation_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/scheduleDelayedSend"
        | "/api/cancelDelayedSend"
        | "/api/postponeDelayedSend"
        | "/api/readDelayedSends" => {
            handle_delayed_send_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/savePinnedPrompt" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, params, _| repository.save_pinned_prompt(params),
        ),
        "/api/saveStashedPrompt" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let result = repository.save_stashed_prompt(params)?;
                if let Some((project_id, session_id)) = result
                    .get("prompt")
                    .and_then(Value::as_object)
                    .and_then(|prompt| {
                        prompt
                            .get("projectId")
                            .and_then(Value::as_str)
                            .zip(prompt.get("sessionId").and_then(Value::as_str))
                    })
                {
                    schedule_presentation_session_delta(
                        &state, db, repository, project_id, session_id,
                    )?;
                }
                Ok(result)
            },
        ),
        "/api/listStashedPrompts" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, params, _| repository.list_stashed_prompts(params),
        ),
        "/api/deleteStashedPrompt" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let result = repository.delete_stashed_prompt(params)?;
                if let Some((project_id, session_id)) = result
                    .get("projectId")
                    .and_then(Value::as_str)
                    .zip(result.get("sessionId").and_then(Value::as_str))
                {
                    schedule_presentation_session_delta(
                        &state, db, repository, project_id, session_id,
                    )?;
                }
                Ok(result)
            },
        ),
        /*
        CDXC:SavedPrompts 2026-08-23:
        The Saved Prompts tag catalogue is daemon-owned like the prompts
        themselves, so every client filters the same rail instead of keeping a
        private list that drifts per machine.
        */
        "/api/listStashedPromptTags" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, _, _| repository.list_stashed_prompt_tags(),
        ),
        "/api/saveStashedPromptTag" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, params, _| repository.save_stashed_prompt_tag(params),
        ),
        "/api/deleteStashedPromptTag" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, params, _| repository.delete_stashed_prompt_tag(params),
        ),
        "/api/setStashedPromptTags" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, params, _| repository.set_stashed_prompt_tags(params),
        ),
        /*
        CDXC:SessionNotes 2026-08-24:
        A saved note changes what the sidebar row renders (the note dot and the
        hover tooltip), so the save schedules the same presentation delta
        `/api/updateSession` does instead of asking every client to refetch.
        The read is a pure lookup and schedules nothing.
        */
        "/api/saveSessionAgentNote" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                let result = repository.save_session_agent_note(params)?;
                /*
                Delta targets come from the repository result (the canonical
                session-row ids), not the raw request params: a coercible
                non-string id in the body would otherwise schedule a bogus
                ("", "") delta that broadcasts `sessionRemoved` to every
                client. Same id source as the /api/createSession arm.
                */
                let project_id = result
                    .get("projectId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let session_id = result
                    .get("sessionId")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                if !project_id.is_empty() && !session_id.is_empty() {
                    schedule_presentation_session_delta(
                        &state,
                        db,
                        repository,
                        &project_id,
                        &session_id,
                    )?;
                }
                Ok(result)
            },
        ),
        "/api/readSessionAgentNote" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _, params, _| repository.read_session_agent_note(params),
        ),
        /*
        CDXC:KeepAwake 2026-08-19:
        A client that is ATTACHED to a session (Ghostex mobile, over its SSH CLI
        bridge) renews a keep-awake lease here so this machine's Auto Sleep sweep
        cannot retire a terminal somebody is actually looking at. The lease lives
        in memory with a TTL — see `session_keep_awake` — and is honored by
        `/api/sleepSession` only for automatic sweeps.
        */
        "/api/readCoordinator"
        | "/api/listCoordinators"
        | "/api/updateCoordinator"
        | "/api/linkCoordinatorThread"
        | "/api/setCoordinatorThreadResolved" => {
            let path = endpoint.path.clone();
            handle_domain_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
                |repository, db, params, _| {
                    coordinator_runtime::handle_coordinator_http(
                        &state, &path, db, repository, params,
                    )
                },
            )
        }
        "/api/toggleCloseAfterDone" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, db, params, _| {
                close_after_done_runtime::toggle_close_after_done(&state, db, repository, params)
            },
        ),
        "/api/holdSessionsAwake" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _db, params, _| hold_sessions_awake(repository, params),
        ),
        "/api/searchSessions" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, params, server_id| search_presentation_sessions(db, server_id, params),
        ),
        "/api/openConversation" => {
            open_conversation_http::handle_open_conversation_http(
                &state,
                endpoint.path,
                request_id,
                body_json,
            )
            .await
        }
        "/api/listPreviousSessions" => {
            let worker_state = state.clone();
            let worker_endpoint = endpoint.path.clone();
            let worker_request_id = request_id.clone();
            match tokio::task::spawn_blocking(move || {
                handle_domain_http(
                    &worker_state,
                    worker_endpoint,
                    worker_request_id,
                    &body_json,
                    |_, db, params, server_id| {
                        crate::external_sessions::discover(
                            db,
                            server_id,
                            &worker_state.paths,
                            params
                                .get("refreshExternalSessions")
                                .and_then(Value::as_bool)
                                == Some(true),
                        )?;
                        list_previous_sessions(db, server_id, params)
                    },
                )
            })
            .await
            {
                Ok(response) => response,
                Err(error) => domain_error_response(
                    endpoint.path,
                    request_id,
                    DomainStateError::corrupt_state(format!("Session discovery failed: {error}")),
                ),
            }
        }
        /*
        CDXC:SessionFork 2026-08-28:
        Previous Sessions hides a closed row once something continues from it, so
        the branch a user forked away from can vanish from every list. This is how
        a client gets the whole family back, ancestors included, to offer a switch
        between branches that share earlier history.
        */
        "/api/sessionForkBranches" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_, db, params, server_id| list_session_fork_branches(db, server_id, params),
        ),
        "/api/agentCliMaintenance" => {
            agent_cli_http::handle(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/managedTools" => {
            managed_tools_http::handle(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/agentbox" => {
            agentbox_http::handle(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/agentAccounts" => {
            accounts_http::handle_accounts_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/readAgentSettings"
        | "/api/updateAgentSettings"
        | "/api/readAgentLaunchPlan"
        | "/api/readAgentResumePlan"
        | "/api/forkSession"
        | "/api/switchDraftAgent"
        | "/api/draftRunLocation"
        | "/api/switchSessionAgent"
        | "/api/requestSessionRename"
        | "/api/cancelFirstPromptAutoTitle"
        | "/api/ingestSessionStateEvent"
        | "/api/ingestTerminalTitleEvent"
        | "/api/updateAgentActivity"
        | "/api/ingestAgentHookEvent" => {
            handle_agent_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/readAgentSkillStatus" | "/api/installAgentSkills" => {
            handle_agent_skill_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/readAgentHookStatus" | "/api/installAgentHooks" | "/api/uninstallAgentHooks" => {
            handle_agent_hook_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/createWorkspaceTerminal"
        | "/api/attachSessionMetadata"
        | "/api/probeSessionProvider"
        | "/api/startSessionProvider"
        | "/api/transitionSession"
        | "/api/sleepSession"
        | "/api/wakeSession"
        | "/api/killSession" => {
            handle_zmx_lifecycle_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/readSessionText"
        | "/api/sendSessionText"
        | "/api/sendSessionMessage"
        | "/api/sendSessionEnter"
        | "/api/focusSession" => {
            handle_zmx_session_interaction_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/dispatchRendererCommand" => {
            handle_renderer_command_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/runGitAction"
        | "/api/runGitHubAction"
        | "/api/runWorktreeAction"
        | "/api/runProjectSetupCommand"
        | "/api/runBeadsAction" => {
            handle_typed_operation_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/runProjectDocsAction" => {
            project_docs_http::handle(state.clone(), endpoint.path, request_id, body_json).await
        }
        "/api/startBoardWork" => {
            handle_board_start_work_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/associateBoardSession" => {
            handle_board_associate_session_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/generateCommitMessage" => {
            handle_generate_commit_message_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/generateSessionTitle" => {
            handle_generate_session_title_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/searchAgentPrompts" => {
            handle_search_agent_prompts_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/readAgentPromptText" => {
            handle_read_agent_prompt_text_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/toggleAgentPromptFavorite" => {
            handle_toggle_agent_prompt_favorite_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/resolveAgentPromptLaunch" => {
            handle_resolve_agent_prompt_launch_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/readSessionChat" => {
            handle_read_session_chat_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/readSessionChatSkills" => {
            handle_read_session_chat_skills_http(&state, endpoint.path, request_id, &body_json)
                .await
        }
        "/api/readSessionChatFiles" => {
            handle_read_session_chat_files_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/sendSessionChatMessage" => {
            let mut response = handle_send_session_chat_message_http(
                &state,
                endpoint.path,
                request_id.clone(),
                &body_json,
            )
            .await;
            crate::session_chat_send_diagnostics::record_response(
                &state,
                &request_id,
                &body_json,
                &mut response,
            )
            .await;
            response
        }
        "/api/saveSessionChatImage" => {
            handle_save_session_chat_image_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/saveSessionChatAttachment" => {
            handle_save_session_chat_attachment_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/readSessionChatImage" => {
            handle_read_session_chat_image_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/answerSessionChatPrompt" => {
            handle_answer_session_chat_prompt_http(&state, endpoint.path, request_id, &body_json)
                .await
        }
        "/api/interruptSessionChat" => {
            handle_interrupt_session_chat_http(&state, endpoint.path, request_id, &body_json).await
        }
        /*
        CDXC:SessionChat 2026-09-02:
        Held open for the whole drive rather than queued-and-acknowledged: the
        answer a caller needs is whether Claude Code ACCEPTED the rewind, and
        that is only knowable once the driver has watched the dialog close.
        */
        "/api/rewindSessionChat" => {
            crate::session_chat_rewind::handle_rewind_session_chat_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
            )
            .await
        }
        "/api/selectSessionChatModel" => {
            crate::session_chat_codex_picker::handle_select_session_chat_model_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
            )
            .await
        }
        "/api/handoffSessionChatDraft" => {
            handle_handoff_session_chat_draft_http(&state, endpoint.path, request_id, &body_json)
                .await
        }
        "/api/replaceSessionChatDraft" => {
            handle_replace_session_chat_draft_http(&state, endpoint.path, request_id, &body_json)
                .await
        }
        "/api/claimSessionChatLaunchDraft" => handle_claim_session_chat_launch_draft_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
        ),
        "/api/readSessionChatQueue"
        | "/api/queueSessionChatPrompt"
        | "/api/updateSessionChatQueuedPrompt"
        | "/api/removeSessionChatQueuedPrompt"
        | "/api/reorderSessionChatQueue"
        | "/api/sendSessionChatQueuedPrompt"
        | "/api/setSessionChatDraft"
        | "/api/acknowledgeSessionChatDraftHandoff" => {
            let mut response = handle_session_chat_queue_http(
                &state,
                endpoint.path,
                request_id.clone(),
                &body_json,
            )
            .await;
            crate::session_chat_send_diagnostics::record_response(
                &state,
                &request_id,
                &body_json,
                &mut response,
            )
            .await;
            response
        }
        // CDXC:Drafts 2026-08-28: the boot-time draft-cache
        // reconcile read; see list_session_chat_drafts_value.
        "/api/listSessionChatDrafts" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |_repository, db, _params, _| {
                crate::session_chat_queue::list_session_chat_drafts_value(db)
            },
        ),
        /*
        CDXC:SessionChat 2026-08-26:
        The evidence read behind a `composerNotReady` refusal. Domain-shaped
        because everything it needs is one session row plus one screen capture,
        and the capture is a direct socket read measured in single-digit
        milliseconds — the same one every screen-state reader already takes.
        */
        "/api/readSessionTerminalTail" => handle_domain_http(
            &state,
            endpoint.path,
            request_id,
            &body_json,
            |repository, _db, params, _| {
                crate::session_chat_composer::read_session_terminal_tail(
                    repository,
                    &read_project_id(params)?,
                    &read_session_id(params)?,
                    params.get("agentId").and_then(Value::as_str),
                )
            },
        ),
        "/api/exportSessionTranscript" => {
            handle_export_session_transcript_http(&state, endpoint.path, request_id, &body_json)
                .await
        }
        "/api/readSessionTranscriptSizes" => {
            handle_read_session_transcript_sizes_http(&state, endpoint.path, request_id, &body_json)
                .await
        }
        "/api/readProjectGitState" => {
            crate::project_git_state::handle_read_project_git_state_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
            )
            .await
        }
        "/api/runGitShipWorkflow" => {
            crate::git_ship_workflow::handle_run_git_ship_workflow_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
            )
            .await
        }
        "/api/createPullRequest" => {
            handle_create_pull_request_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/queryLogs" => handle_query_logs_http(&state, endpoint.path, request_id, &body_json),
        "/api/updatePortlessState" => {
            handle_portless_state_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/tailcatStatus" | "/api/updateTailcatState" | "/api/installTailcat" => {
            handle_tailcat_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/remoteAccessStatus"
        | "/api/enableSshAccess"
        | "/api/remotePairingCode"
        | "/api/pairedDevices"
        | "/api/removePairedDevice"
        | "/api/pairDevice"
        | "/api/pairedDeviceSeen" => {
            handle_remote_access_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/previewRepositoryClone"
        | "/api/startRepositoryClone"
        | "/api/readRepositoryCloneJob"
        | "/api/cancelRepositoryCloneJob" => {
            handle_repository_clone_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/browseProjectDirectories" => {
            handle_browse_project_directories_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/createProjectDirectory" => {
            handle_create_project_directory_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/discoverSourceControl" | "/api/lookupRepository" => {
            handle_source_control_http(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/resolveGitRootForPath" => {
            handle_resolve_git_root_for_path_http(&state, endpoint.path, request_id, &body_json)
        }
        "/api/projectView" => {
            crate::project_views::handle(&state, endpoint.path, request_id, &body_json).await
        }
        "/api/listExtensions"
        | "/api/extensionsCatalog"
        | "/api/installExtension"
        | "/api/uninstallExtension"
        | "/api/updateExtensionState"
        | "/api/startExtension"
        | "/api/stopExtension"
        | "/api/extensionStatus"
        | "/api/extensionBadge" => {
            handle_extensions_http(
                &state,
                endpoint.path,
                request_id,
                &body_json,
                token_extension_id,
            )
            .await
        }
        "/api/control/stop" => {
            broadcast_server_stopping(&state);
            let response = routed_json(
                Some(endpoint.path),
                StatusCode::OK,
                rpc_success(request_id, json!({})),
            );
            let _ = state.shutdown_tx.send(());
            response
        }
        "/api/control/stopAll" => {
            broadcast_server_stopping(&state);
            /*
            CDXC:SessionSleep 2026-08-28:
            Kill the tracked zmx sessions BEFORE answering and before the
            shutdown broadcast, so the reported counts are real and the kills
            cannot race the control plane's own teardown. The kill loop runs
            zmx subprocesses per session, so it leaves the async executor.
            */
            let stop_all_state = state.clone();
            let counts = tokio::task::spawn_blocking(move || {
                zmx_http::kill_all_tracked_zmx_sessions(&stop_all_state)
            })
            .await
            .unwrap_or(zmx_http::ZmxStopAllCounts {
                attempted: 0,
                failed: 0,
                killed: 0,
                skipped: 0,
            });
            let response = routed_json(
                Some(endpoint.path),
                StatusCode::OK,
                rpc_success(
                    request_id,
                    json!({
                        "attemptedSessions": counts.attempted,
                        "failedSessions": counts.failed,
                        "killedSessions": counts.killed,
                        "skippedSessions": counts.skipped,
                    }),
                ),
            );
            let _ = state.shutdown_tx.send(());
            response
        }
        _ => routed_json(
            Some(endpoint.path.clone()),
            StatusCode::NOT_IMPLEMENTED,
            rpc_error(
                "notImplemented",
                format!(
                    "{} is defined but not implemented in this milestone.",
                    endpoint.path
                ),
                Some(request_id),
            ),
        ),
    }
}
