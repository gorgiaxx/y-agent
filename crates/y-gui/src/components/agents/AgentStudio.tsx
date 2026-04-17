import { Plus } from 'lucide-react';
import { Button } from '../ui/Button';
import { StatusBar } from '../chat-panel/StatusBar';
import { ChatPanel } from '../chat-panel/ChatPanel';
import { InputArea } from '../chat-panel/input-area/InputArea';
import type { AgentInfo, AgentDetail } from '../../hooks/useAgents';
import type { SessionInfo } from '../../types';
import type { Message, ThinkingEffort, PlanMode, McpMode, Attachment, SkillInfo, KnowledgeCollectionInfo, ProviderInfo } from '../../types';
import type { ToolResultRecord } from '../../hooks/chatStreamTypes';
import type { InterleavedSegment } from '../../hooks/useInterleavedSegments';
import type { PendingEdit, CompactInfo } from '../../hooks/useChat';
import type { AskUserDialogState, PermissionDialogState } from '../../hooks/useSessionInteractions';
import { AgentSessionRail } from './AgentSessionRail';

interface AgentStudioProps {
  agentSummary: AgentInfo | AgentDetail | null;
  agentId: string;
  detailLoading: boolean;
  sessions: SessionInfo[];
  activeSessionId: string | null;
  sessionsLoading: boolean;
  streamingSessionIds: Set<string>;
  messages: Message[];
  isStreaming: boolean;
  isLoadingMessages: boolean;
  error: string | null;
  toolResults: ToolResultRecord[];
  getStreamSegments: () => InterleavedSegment[] | null;
  contextResetPoints: number[];
  compactPoints: CompactInfo[];
  providerCount: number;
  version: string;
  activeModel: string | undefined;
  activeProviderIcon: string | null;
  lastTokens: { input: number; output: number } | undefined;
  lastCost: number | undefined;
  contextWindow: number | undefined;
  contextTokensUsed: number | undefined;
  selectedProviderId: string;
  thinkingEffort: ThinkingEffort | null;
  planMode: PlanMode;
  inputDisabled: boolean;
  sendOnEnter: boolean;
  providers: ProviderInfo[];
  providerIcons: Record<string, string>;
  visibleSkills: SkillInfo[];
  visibleKnowledge: KnowledgeCollectionInfo[];
  inputExpanded: boolean;
  pendingEdit: PendingEdit | null;
  isCompacting: boolean;
  hasCustomPrompt: boolean;
  rewindDraft: string | null;
  mcpMode: McpMode;
  onMcpModeChange: (mode: McpMode) => void;
  mcpServerList: { name: string; disabled: boolean }[];
  selectedMcpServers: string[];
  onMcpServerToggle: (name: string) => void;
  askUserData: AskUserDialogState | null;
  permissionData: PermissionDialogState | null;
  onEdit: () => void;
  onNewSession: () => void;
  onSelectSession: (id: string) => void;
  onDeleteSession: (id: string) => void;
  // ChatPanel callbacks (content first, then messageId)
  onEditMessage: (content: string, messageId: string) => void;
  onUndoMessage: (messageId: string) => void;
  onResendMessage: (content: string, messageId: string) => void;
  onRestoreBranch: (checkpointId: string) => void;
  onForkMessage: (messageIndex: number) => void;
  // InputArea callbacks
  onSend: (message: string, skills?: string[], knowledgeCollections?: string[], thinkingEffort?: ThinkingEffort | null, attachments?: Attachment[], planMode?: PlanMode, mcpMode?: McpMode | null, mcpServers?: string[]) => void;
  onStop: () => void;
  onCommand: (command: string) => boolean;
  onSelectProvider: (id: string) => void;
  onThinkingEffortChange: (effort: ThinkingEffort | null) => void;
  onPlanModeChange: (mode: PlanMode) => void;
  onExpandChange: (expanded: boolean) => void;
  onCancelEdit: () => void;
  onClearSession: () => void;
  onAddContextReset: () => void;
  onCustomPromptChange: (hasPrompt: boolean) => void;
  onRewindDraftConsumed: () => void;
  onAskUserSubmit: (interactionId: string, answers: Record<string, string>) => void;
  onAskUserDismiss: (interactionId: string) => void;
  onPermissionApprove: (requestId: string) => void;
  onPermissionDeny: (requestId: string) => void;
  onPermissionAllowAllForSession: (requestId: string) => void;
}

export function AgentStudio({
  agentSummary,
  agentId,
  detailLoading,
  sessions,
  activeSessionId,
  sessionsLoading,
  streamingSessionIds,
  messages,
  isStreaming,
  isLoadingMessages,
  error,
  toolResults,
  getStreamSegments,
  contextResetPoints,
  compactPoints,
  providerCount,
  version,
  activeModel,
  activeProviderIcon,
  lastTokens,
  lastCost,
  contextWindow,
  contextTokensUsed,
  selectedProviderId,
  thinkingEffort,
  planMode,
  inputDisabled,
  sendOnEnter,
  providers,
  providerIcons,
  visibleSkills,
  visibleKnowledge,
  inputExpanded,
  pendingEdit,
  isCompacting,
  hasCustomPrompt,
  rewindDraft,
  mcpMode,
  onMcpModeChange,
  mcpServerList,
  selectedMcpServers,
  onMcpServerToggle,
  askUserData,
  permissionData,
  onEdit,
  onNewSession,
  onSelectSession,
  onDeleteSession,
  onEditMessage,
  onUndoMessage,
  onResendMessage,
  onRestoreBranch,
  onForkMessage,
  onSend,
  onStop,
  onCommand,
  onSelectProvider,
  onThinkingEffortChange,
  onPlanModeChange,
  onExpandChange,
  onCancelEdit,
  onClearSession,
  onAddContextReset,
  onCustomPromptChange,
  onRewindDraftConsumed,
  onAskUserSubmit,
  onAskUserDismiss,
  onPermissionApprove,
  onPermissionDeny,
  onPermissionAllowAllForSession,
}: AgentStudioProps) {
  return (
    <div className="agents-studio">
      <AgentSessionRail
        sessions={sessions}
        activeSessionId={activeSessionId}
        loading={sessionsLoading}
        streamingSessionIds={streamingSessionIds}
        onEdit={onEdit}
        onNewSession={onNewSession}
        onSelectSession={onSelectSession}
        onDeleteSession={onDeleteSession}
      />

      <section className="agents-chat-stage">
        {detailLoading ? (
          <div className="agents-chat-stage-empty">
            Loading agent preset...
          </div>
        ) : activeSessionId ? (
          <div className="agents-chat-stage-body">
            <ChatPanel
              messages={messages}
              isStreaming={isStreaming}
              isLoading={isLoadingMessages}
              error={error}
              onEditMessage={onEditMessage}
              onUndoMessage={onUndoMessage}
              onResendMessage={onResendMessage}
              onForkMessage={onForkMessage}
              onRestoreBranch={onRestoreBranch}
              toolResults={toolResults}
              getStreamSegments={getStreamSegments}
              contextResetPoints={contextResetPoints}
              compactPoints={compactPoints}
            />
          </div>
        ) : (
          <div className="agents-chat-stage-empty">
            <div className="agents-chat-empty-card">
              <div className="agents-chat-empty-title">
                {agentSummary?.name ?? 'Agent'}
              </div>
              <p className="agents-chat-empty-description">
                Start a dedicated session for this preset to keep prompts, tools, and
                retrieval settings aligned from the first turn.
              </p>
              <Button variant="primary" size="sm" onClick={onNewSession}>
                <Plus size={12} />
                New Session
              </Button>
            </div>
          </div>
        )}

        <InputArea
          key={activeSessionId ?? `${agentId}-empty`}
          onSend={onSend}
          onStop={onStop}
          onCommand={onCommand}
          disabled={inputDisabled}
          sendOnEnter={sendOnEnter}
          providers={providers}
          selectedProviderId={selectedProviderId}
          onSelectProvider={onSelectProvider}
          pendingEdit={pendingEdit}
          onCancelEdit={onCancelEdit}
          skills={visibleSkills}
          knowledgeCollections={visibleKnowledge}
          expanded={inputExpanded}
          onExpandChange={onExpandChange}
          onClearSession={onClearSession}
          onAddContextReset={onAddContextReset}
          providerIcons={providerIcons}
          thinkingEffort={thinkingEffort}
          onThinkingEffortChange={onThinkingEffortChange}
          planMode={planMode}
          onPlanModeChange={onPlanModeChange}
          persistPlanMode={false}
          askUserData={askUserData}
          onAskUserSubmit={onAskUserSubmit}
          onAskUserDismiss={onAskUserDismiss}
          permissionData={permissionData}
          onPermissionApprove={onPermissionApprove}
          onPermissionDeny={onPermissionDeny}
          onPermissionAllowAllForSession={onPermissionAllowAllForSession}
          isCompacting={isCompacting}
          sessionId={activeSessionId}
          hasCustomPrompt={hasCustomPrompt}
          onCustomPromptChange={onCustomPromptChange}
          rewindDraft={rewindDraft}
          onRewindDraftConsumed={onRewindDraftConsumed}
          mcpMode={mcpMode}
          onMcpModeChange={onMcpModeChange}
          mcpServerList={mcpServerList}
          selectedMcpServers={selectedMcpServers}
          onMcpServerToggle={onMcpServerToggle}
        />

        <StatusBar
          providerCount={providerCount}
          sessionCount={sessions.length}
          version={version}
          activeModel={activeModel}
          activeProviderIcon={activeProviderIcon}
          lastCost={lastCost}
          lastTokens={lastTokens}
          contextWindow={contextWindow}
          contextTokensUsed={contextTokensUsed}
        />
      </section>
    </div>
  );
}
