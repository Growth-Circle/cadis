//! Orchestrator routing, spawn directives, and message dispatch.

use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RouteDecision {
    pub(crate) agent_id: AgentId,
    pub(crate) agent_name: String,
    pub(crate) content: String,
    pub(crate) reason: String,
    pub(crate) worker_summary: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SpawnRouteDecision {
    pub(crate) role: String,
    pub(crate) content: String,
    pub(crate) reason: String,
    pub(crate) worker_summary: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OrchestratorDecision {
    Route(RouteDecision),
    SpawnAndRoute(SpawnRouteDecision),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ExplicitOrchestratorAction {
    Route {
        mention: String,
        content: String,
        worker_summary: String,
    },
    Spawn {
        role: String,
        content: String,
        worker_summary: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Orchestrator {
    pub(crate) config: OrchestratorConfig,
}

impl Orchestrator {
    pub(crate) fn new(config: OrchestratorConfig) -> Self {
        Self { config }
    }

    pub(crate) fn route_message(
        &self,
        explicit_agent_id: Option<AgentId>,
        content: &str,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> Result<OrchestratorDecision, RouteError> {
        let route = if let Some(agent_id) = explicit_agent_id {
            let content = self.strip_matching_leading_mention(&agent_id, content, agents);
            self.route_to_agent(
                agent_id,
                content,
                "explicit target_agent_id".to_owned(),
                None,
                agents,
            )?
        } else if let Some((mention, remaining)) = leading_mention(content) {
            let Some(agent_id) = self.resolve_agent_mention(&mention, agents) else {
                return Err(RouteError {
                    code: "agent_not_found",
                    message: format!("no agent matches @{mention}"),
                });
            };
            self.route_to_agent(
                agent_id,
                remaining,
                format!("@{mention} mention"),
                None,
                agents,
            )?
        } else {
            self.route_to_agent(
                AgentId::from("main"),
                content.to_owned(),
                "default orchestrator".to_owned(),
                None,
                agents,
            )?
        };

        if route.agent_id.as_str() == "main" {
            self.apply_explicit_action(route, agents)
        } else {
            Ok(OrchestratorDecision::Route(route))
        }
    }

    pub(crate) fn apply_explicit_action(
        &self,
        route: RouteDecision,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> Result<OrchestratorDecision, RouteError> {
        let Some(action) = parse_orchestrator_action(&route.content, &self.config) else {
            return Ok(OrchestratorDecision::Route(route));
        };

        if !self.config.worker_delegation_enabled {
            return Err(RouteError {
                code: "orchestrator_worker_delegation_disabled",
                message: "orchestrator worker delegation actions are disabled".to_owned(),
            });
        }

        match action {
            ExplicitOrchestratorAction::Route {
                mention,
                content,
                worker_summary,
            } => {
                let Some(agent_id) = self.resolve_agent_mention(&mention, agents) else {
                    return Err(RouteError {
                        code: "agent_not_found",
                        message: format!("no agent matches @{mention}"),
                    });
                };
                self.route_to_agent(
                    agent_id,
                    content,
                    format!("orchestrator action: route @{mention}"),
                    Some(worker_summary),
                    agents,
                )
                .map(OrchestratorDecision::Route)
            }
            ExplicitOrchestratorAction::Spawn {
                role,
                content,
                worker_summary,
            } => Ok(OrchestratorDecision::SpawnAndRoute(SpawnRouteDecision {
                role,
                content,
                reason: "orchestrator action: spawn worker".to_owned(),
                worker_summary,
            })),
        }
    }

    pub(crate) fn route_to_agent(
        &self,
        agent_id: AgentId,
        content: String,
        reason: String,
        worker_summary: Option<String>,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> Result<RouteDecision, RouteError> {
        let Some(agent) = agents.get(&agent_id) else {
            return Err(RouteError {
                code: "agent_not_found",
                message: format!("agent '{agent_id}' was not found"),
            });
        };
        Ok(RouteDecision {
            agent_id: agent.id.clone(),
            agent_name: agent.display_name.clone(),
            content: normalize_route_content(content),
            reason,
            worker_summary,
        })
    }

    pub(crate) fn resolve_agent_mention(
        &self,
        mention: &str,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> Option<AgentId> {
        let normalized = normalize_lookup(mention);
        agents
            .values()
            .find(|agent| {
                [
                    agent.id.as_str(),
                    agent.display_name.as_str(),
                    agent.role.as_str(),
                ]
                .into_iter()
                .any(|candidate| normalize_lookup(candidate) == normalized)
            })
            .map(|agent| agent.id.clone())
    }

    pub(crate) fn strip_matching_leading_mention(
        &self,
        agent_id: &AgentId,
        content: &str,
        agents: &HashMap<AgentId, AgentRecord>,
    ) -> String {
        let Some((mention, remaining)) = leading_mention(content) else {
            return content.to_owned();
        };
        if self.resolve_agent_mention(&mention, agents).as_ref() == Some(agent_id) {
            remaining
        } else {
            content.to_owned()
        }
    }
}

pub(crate) fn normalize_role(value: &str) -> String {
    let collapsed: String = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return collapsed;
    }
    collapsed
        .parse::<cadis_protocol::AgentRole>()
        .map(|r| r.as_str().to_owned())
        .unwrap_or(collapsed)
}

pub(crate) fn leading_mention(content: &str) -> Option<(String, String)> {
    let trimmed = content.trim_start();
    let rest = trimmed.strip_prefix('@')?;
    let mut end = 0;
    for (index, character) in rest.char_indices() {
        if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
            end = index + character.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 {
        return None;
    }
    let mention = rest[..end].to_owned();
    let remaining = rest[end..].trim_start().to_owned();
    Some((mention, remaining))
}

pub(crate) fn parse_orchestrator_action(
    content: &str,
    config: &OrchestratorConfig,
) -> Option<ExplicitOrchestratorAction> {
    let trimmed = content.trim_start();
    let (command, rest) = trimmed
        .split_once(char::is_whitespace)
        .unwrap_or((trimmed, ""));
    let command = command.strip_prefix('/')?.to_ascii_lowercase();
    let rest = rest.trim_start();

    match command.as_str() {
        "route" | "delegate" => {
            let (mention, remaining) = leading_mention(rest)?;
            let content = normalize_route_content(remaining);
            let worker_summary = format!("Route @{mention}: {}", summarize_task(&content));
            Some(ExplicitOrchestratorAction::Route {
                mention,
                content,
                worker_summary,
            })
        }
        "spawn" => {
            let (role, content) = parse_spawn_action(rest, true, &config.default_worker_role)?;
            let content = normalize_route_content(content);
            let worker_summary = format!("Spawn {role}: {}", summarize_task(&content));
            Some(ExplicitOrchestratorAction::Spawn {
                role,
                content,
                worker_summary,
            })
        }
        "worker" => {
            let (role, content) = parse_spawn_action(rest, false, &config.default_worker_role)?;
            let content = normalize_route_content(content);
            let worker_summary = format!("Worker {role}: {}", summarize_task(&content));
            Some(ExplicitOrchestratorAction::Spawn {
                role,
                content,
                worker_summary,
            })
        }
        _ => None,
    }
}

pub(crate) fn parse_spawn_action(
    rest: &str,
    require_role: bool,
    default_worker_role: &str,
) -> Option<(String, String)> {
    let rest = rest.trim();
    if rest.is_empty() {
        if require_role {
            return None;
        }
        return Some((default_worker_role.to_owned(), String::new()));
    }

    if let Some((role, content)) = rest.split_once(':') {
        let role = normalize_role(role);
        if role.is_empty() {
            return None;
        }
        return Some((role, content.trim_start().to_owned()));
    }

    if require_role {
        Some((normalize_role(rest), String::new()))
    } else {
        Some((default_worker_role.to_owned(), rest.to_owned()))
    }
}

pub(crate) fn normalize_route_content(content: String) -> String {
    let content = content.trim().to_owned();
    if content.is_empty() {
        "Continue.".to_owned()
    } else {
        content
    }
}

pub(crate) fn summarize_task(content: &str) -> String {
    let summary = content
        .split_whitespace()
        .take(12)
        .collect::<Vec<_>>()
        .join(" ");
    if summary.is_empty() {
        "Continue.".to_owned()
    } else {
        summary
    }
}

/// Parsed model-driven spawn directive from a model response.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ModelSpawnDirective {
    pub(crate) role: String,
    pub(crate) task: String,
}

/// Maximum spawn directives parsed from a single model response.
pub(crate) const MAX_SPAWN_DIRECTIVES_PER_RESPONSE: usize = 3;
/// Maximum length for a model-driven spawn role.
pub(crate) const SPAWN_ROLE_MAX_LEN: usize = 64;
/// Maximum length for a model-driven spawn task.
pub(crate) const SPAWN_TASK_MAX_LEN: usize = 256;

/// Parses `[SPAWN role: task]` directives from model response text.
pub(crate) fn parse_model_spawn_directives(content: &str) -> Vec<ModelSpawnDirective> {
    let mut directives = Vec::new();
    for line in content.lines() {
        if directives.len() >= MAX_SPAWN_DIRECTIVES_PER_RESPONSE {
            break;
        }
        let trimmed = line.trim();
        if let Some(inner) = trimmed
            .strip_prefix("[SPAWN ")
            .and_then(|s| s.strip_suffix(']'))
        {
            if let Some((role, task)) = inner.split_once(':') {
                let role = truncate_to_utf8_boundary(&normalize_role(role), SPAWN_ROLE_MAX_LEN)
                    .0
                    .to_owned();
                let task = truncate_to_utf8_boundary(task.trim(), SPAWN_TASK_MAX_LEN)
                    .0
                    .to_owned();
                if !role.is_empty() && !task.is_empty() {
                    directives.push(ModelSpawnDirective { role, task });
                }
            }
        }
    }
    directives
}
