const API_BASE =
  (typeof window !== "undefined" && window.__WINDIE_API_URL__) ||
  process.env.REACT_APP_WINDIE_API_URL ||
  "http://127.0.0.1:8787";
function parseApiBody(text) {
  if (!text) return null;
  try {
    return JSON.parse(text);
  } catch {
    return { error: text };
  }
}

export async function apiRequest(path, options = {}) {
  const { headers: optionHeaders = {}, ...fetchOptions } = options;
  const response = await fetch(`${API_BASE}${path}`, {
    ...fetchOptions,
    headers: {
      "Content-Type": "application/json",
      ...optionHeaders,
    },
  });

  const text = await response.text();
  const body = parseApiBody(text);

  if (!response.ok) {
    throw new Error(body?.error || `Windie API request failed: ${response.status}`);
  }

  return body;
}

export async function fetchImageAsset(conversationId, assetId) {
  const response = await fetch(
    `${API_BASE}/api/conversations/${encodeURIComponent(conversationId)}/images/${encodeURIComponent(assetId)}`,
    {
    }
  );

  if (!response.ok) {
    const text = await response.text();
    const body = parseApiBody(text);
    throw new Error(body?.error || `Windie image request failed: ${response.status}`);
  }

  return response.blob();
}

export async function listModels() {
  const body = await apiRequest("/api/models");
  return (body.models || []).map((model) => ({
    id: model.id,
    label: model.id,
    contextLength: model.context_length ?? null,
    maxInputTokens: model.max_input_tokens ?? null,
    maxOutputTokens: model.max_output_tokens ?? null,
  }));
}

export async function countConversationInputTokens(conversationId, model = null, headMessageId = null) {
  const body = await apiRequest(
    `/api/conversations/${encodeURIComponent(conversationId)}/input-tokens`,
    {
      method: "POST",
      body: JSON.stringify({
        model: model || null,
        head_message_id: headMessageId || null,
      }),
    }
  );

  return {
    inputTokens: body?.input_tokens ?? null,
    totalTokens: body?.total_tokens ?? null,
    model: body?.model ?? null,
    source: body?.source || null,
    raw: body?.raw || null,
  };
}

export async function fetchModelParameters(model) {
  return apiRequest(`/api/model-parameters?model=${encodeURIComponent(model)}`);
}

export async function createSession(conversationId, body = {}) {
  return apiRequest(`/api/conversations/${encodeURIComponent(conversationId)}/sessions`, {
    method: "POST",
    body: JSON.stringify({
      head_message_id: body.headMessageId || null,
      model: body.model || null,
      reasoning: body.reasoning || null,
    }),
  });
}

export async function listSessions() {
  const body = await apiRequest("/api/sessions");
  return body.sessions || [];
}

export async function listConversationSessions(conversationId) {
  const body = await apiRequest(
    `/api/conversations/${encodeURIComponent(conversationId)}/sessions`
  );
  return body.sessions || [];
}

export async function resolveSessionAtHead(conversationId, headMessageId = null) {
  return apiRequest(
    `/api/conversations/${encodeURIComponent(conversationId)}/sessions/resolve`,
    {
      method: "POST",
      body: JSON.stringify({ head_message_id: headMessageId || null }),
    }
  );
}

export async function queryConversation(conversationId, body = {}) {
  return apiRequest(`/api/conversations/${encodeURIComponent(conversationId)}/query`, {
    method: "POST",
    body: JSON.stringify({
      head_message_id: body.headMessageId || null,
      text: body.text || null,
      parts: body.parts || [],
    }),
  });
}

export async function continueConversation(conversationId, headMessageId = null) {
  return apiRequest(`/api/conversations/${encodeURIComponent(conversationId)}/continue`, {
    method: "POST",
    body: JSON.stringify({ head_message_id: headMessageId || null }),
  });
}

export async function querySession(sessionId, parts) {
  return apiRequest(`/api/sessions/${encodeURIComponent(sessionId)}/query`, {
    method: "POST",
    body: JSON.stringify({ parts }),
  });
}

export async function continueSession(sessionId) {
  return apiRequest(`/api/sessions/${encodeURIComponent(sessionId)}/continue`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function getSession(sessionId) {
  return apiRequest(`/api/sessions/${encodeURIComponent(sessionId)}`);
}

export async function deleteSession(sessionId) {
  return apiRequest(`/api/sessions/${encodeURIComponent(sessionId)}`, {
    method: "DELETE",
  });
}

export async function stopSession(sessionId) {
  return apiRequest(`/api/sessions/${encodeURIComponent(sessionId)}/stop`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function approveSessionTool(sessionId, toolCallId) {
  return apiRequest(
    `/api/sessions/${encodeURIComponent(sessionId)}/approvals/${encodeURIComponent(toolCallId)}/approve`,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
}

export async function denySessionTool(sessionId, toolCallId) {
  return apiRequest(
    `/api/sessions/${encodeURIComponent(sessionId)}/approvals/${encodeURIComponent(toolCallId)}/deny`,
    {
      method: "POST",
      body: JSON.stringify({}),
    }
  );
}

export async function listProviderInstallations() {
  const body = await apiRequest("/api/providers");
  return Array.isArray(body) ? body : body.providers || [];
}

export async function listLlmProviders() {
  const body = await apiRequest("/api/llm/providers");
  return body.providers || [];
}

export async function listLlmProviderKeys(provider) {
  const body = await apiRequest(
    `/api/llm/providers/${encodeURIComponent(provider)}/keys`
  );
  return body.keys || [];
}

export async function ensureLlmProvider(provider) {
  return apiRequest(`/api/llm/providers/${encodeURIComponent(provider)}/ensure`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function createLlmProviderKey(provider, { name, value }) {
  return apiRequest(`/api/llm/providers/${encodeURIComponent(provider)}/keys`, {
    method: "POST",
    body: JSON.stringify({
      name,
      value,
      models: ["*"],
      blacklisted_models: [],
      weight: 1.0,
      enabled: true,
    }),
  });
}

export async function deleteLlmProviderKey(provider, keyId) {
  return apiRequest(
    `/api/llm/providers/${encodeURIComponent(provider)}/keys/${encodeURIComponent(keyId)}`,
    { method: "DELETE" }
  );
}

export async function setEnvValues(assignments) {
  return apiRequest("/api/env", {
    method: "PUT",
    body: JSON.stringify({ assignments }),
  });
}

export async function setupProvider(providerId) {
  return apiRequest(`/api/providers/${encodeURIComponent(providerId)}/setup`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function enableProvider(providerId) {
  return apiRequest(`/api/providers/${encodeURIComponent(providerId)}/enable`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function disableProvider(providerId) {
  return apiRequest(`/api/providers/${encodeURIComponent(providerId)}/disable`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function repairProvider(providerId) {
  return apiRequest(`/api/providers/${encodeURIComponent(providerId)}/repair`, {
    method: "POST",
    body: JSON.stringify({}),
  });
}

export async function uninstallProvider(providerId) {
  return apiRequest(`/api/providers/${encodeURIComponent(providerId)}`, {
    method: "DELETE",
  });
}

export async function setConversationModel(conversationId, model) {
  return apiRequest(`/api/conversations/${encodeURIComponent(conversationId)}/model`, {
    method: "PATCH",
    body: JSON.stringify({ model }),
  });
}

export async function setConversationReasoning(conversationId, effort) {
  return apiRequest(`/api/conversations/${encodeURIComponent(conversationId)}/reasoning`, {
    method: "PATCH",
    body: JSON.stringify({ effort: effort || null }),
  });
}
