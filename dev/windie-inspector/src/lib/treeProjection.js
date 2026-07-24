/**
 * Builds the visual-tree projection without changing the stored conversation.
 * Assistant tool-call messages and tool outputs are grouped into expandable
 * presentation nodes so the tree emphasizes user messages and final replies.
 */

export function isExecutionNode(node) {
  const role = node?.message?.role;
  return role === "tool" || (role === "assistant" && node.message.metadata?.toolCalls?.length > 0);
}

/**
 * Counts actual tool invocations represented by an execution group.
 *
 * An execution group normally contains both an assistant tool-call message
 * and its tool result. Counting nodes would therefore count one invocation
 * twice. The two persisted representations are reconciled by taking the
 * larger of the tool-call and tool-result counts.
 */
export function executionToolCount(nodes) {
  const assistantCalls = nodes.reduce(
    (count, node) =>
      count + (node?.message?.role === "assistant" ? node.message.metadata?.toolCalls?.length || 0 : 0),
    0
  );
  const toolResults = nodes.filter((node) => node?.message?.role === "tool").length;
  return Math.max(assistantCalls, toolResults);
}

function groupId(startId) {
  return `execution-group:${startId}`;
}

function collectExecutionSubtree(nodes, startId) {
  const hiddenIds = [];
  const frontierIds = [];

  const visit = (id) => {
    const node = nodes[id];
    if (!node) return;
    if (!isExecutionNode(node)) {
      frontierIds.push(id);
      return;
    }
    hiddenIds.push(id);
    (node.childrenIds || []).forEach(visit);
  };

  visit(startId);
  return {
    hiddenIds,
    frontierIds,
    toolCount: executionToolCount(hiddenIds.map((id) => nodes[id])),
  };
}

/**
 * Replaces each collapsed execution subtree with one visual group node.
 * Expanded group ids are presentation-only ids returned by this function.
 */
export function projectTree(conversation, expandedGroupIds = new Set()) {
  if (!conversation) return { nodes: {}, rootIds: [] };

  const sourceNodes = conversation.nodes || {};
  const projectedNodes = {};
  const rootIds = [];

  const addMessage = (id, parentId = null) => {
    const source = sourceNodes[id];
    if (!source || projectedNodes[id]) return;
    projectedNodes[id] = {
      ...source,
      kind: "message",
      originalId: id,
      parentId,
      childrenIds: [],
    };
    if (parentId) projectedNodes[parentId]?.childrenIds.push(id);
    else rootIds.push(id);
  };

  const addGroup = (startId, parentId, hiddenIds, frontierIds, toolCount, expanded = false) => {
    const id = groupId(startId);
    projectedNodes[id] = {
      id,
      kind: "execution_group",
      expanded,
      originalId: null,
      parentId,
      childrenIds: [],
      hiddenIds,
      frontierIds,
      toolCount,
    };
    if (parentId) projectedNodes[parentId]?.childrenIds.push(id);
    else rootIds.push(id);
    return id;
  };

  const renderVisible = (id, parentId = null) => {
    addMessage(id, parentId);
    const node = sourceNodes[id];
    if (!node) return;
    (node.childrenIds || []).forEach((childId) => {
      if (isExecutionNode(sourceNodes[childId])) {
        renderExecutionGroup(childId, id);
      } else {
        renderVisible(childId, id);
      }
    });
  };

  const renderExpandedExecution = (id, parentId) => {
    addMessage(id, parentId);
    const node = sourceNodes[id];
    if (!node) return;
    (node.childrenIds || []).forEach((childId) => {
      if (isExecutionNode(sourceNodes[childId])) {
        renderExpandedExecution(childId, id);
      } else {
        renderVisible(childId, id);
      }
    });
  };

  function renderExecutionGroup(startId, parentId) {
    const { hiddenIds, frontierIds, toolCount } = collectExecutionSubtree(sourceNodes, startId);
    const id = groupId(startId);
    if (expandedGroupIds.has(id)) {
      const groupNodeId = addGroup(startId, parentId, hiddenIds, frontierIds, toolCount, true);
      renderExpandedExecution(startId, groupNodeId);
      return;
    }

    const groupNodeId = addGroup(startId, parentId, hiddenIds, frontierIds, toolCount);
    frontierIds.forEach((childId) => renderVisible(childId, groupNodeId));
  }

  const sourceRootIds = conversation.rootIds?.length
    ? conversation.rootIds
    : Object.values(sourceNodes)
        .filter((node) => node.parentId === null)
        .map((node) => node.id);

  sourceRootIds.forEach((rootId) => {
    if (isExecutionNode(sourceNodes[rootId])) {
      renderExecutionGroup(rootId, null);
    } else {
      renderVisible(rootId);
    }
  });

  return { nodes: projectedNodes, rootIds };
}

export function isExecutionGroup(node) {
  return node?.kind === "execution_group";
}
