import { Plus } from 'lucide-react';
import { Button } from '../ui/Button';
import { StatusBar } from '../chat-panel/StatusBar';
import { ChatPanel } from '../chat-panel/ChatPanel';
import { InputArea } from '../chat-panel/input-area/InputArea';
import type { InputProviderProps, InputMcpProps, InputDialogProps, InputEditProps, InputFeatureProps } from '../chat-panel/input-area/InputArea';
import type { AgentInfo, AgentDetail } from '../../hooks/useAgents';
import type { SessionInfo } from '../../types';
import type { Message, ThinkingEffort, PlanMode, McpMode, Attachment, SkillInfo, KnowledgeCollectionInfo } from '../../types';
import type { ToolResultRecord } from '../../hooks/chatStreamTypes';
import type { InterleavedSegment } from '../../hooks/useInterleavedSegments';
import type { CompactInfo } from '../../hooks/useChat';
import { useBackgroundTasksContext, useViewRouting } from '../../providers/AppContexts';

interface AgentStudioProps {
  agentSummary: AgentInfo | AgentDetail | null;
  agentId: string;
  detailLoading: boolean;
  sessions: SessionInfo[];
  activeSessionId: string | null;
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
  inputDisabled: boolean;
  sendOnEnter: boolean;
  visibleSkills: SkillInfo[];
  visibleKnowledge: KnowledgeCollectionInfo[];
  inputExpanded: boolean;
  isCompacting: boolean;
  hasCustomPrompt: boolean;
  onNewSession: () => void;
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
  onExpandChange: (expanded: boolean) => void;
  onClearSession: () => void;
  onAddContextReset: () => void;
  onCustomPromptChange: (hasPrompt: boolean) => void;
  // Grouped InputArea prop objects
  provider: InputProviderProps;
  mcp: InputMcpProps;
  dialogs: InputDialogProps;
  edit: InputEditProps;
  features: InputFeatureProps;
}

export function AgentStudio({
  agentSummary,
  agentId,
  detailLoading,
  sessions,
  activeSessionId,
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
  inputDisabled,
  sendOnEnter,
  visibleSkills,
  visibleKnowledge,
  inputExpanded,
  isCompacting,
  hasCustomPrompt,
  onNewSession,
  onEditMessage,
  onUndoMessage,
  onResendMessage,
  onRestoreBranch,
  onForkMessage,
  onSend,
  onStop,
  onCommand,
  onExpandChange,
  onClearSession,
  onAddContextReset,
  onCustomPromptChange,
  provider,
  mcp,
  dialogs,
  edit,
  features,
}: AgentStudioProps) {
  const backgroundTasks = useBackgroundTasksContext();
  const viewRouting = useViewRouting();
  const backgroundTaskTotal = backgroundTasks.tasks.length;
  const backgroundTaskRunning = backgroundTasks.tasks.filter((task) => task.status === 'running').length;
  const backgroundTaskFailed = backgroundTasks.tasks.filter((task) => task.status === 'failed').length;

  return (
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
        skills={visibleSkills}
        knowledgeCollections={visibleKnowledge}
        expanded={inputExpanded}
        onExpandChange={onExpandChange}
        onClearSession={onClearSession}
        onAddContextReset={onAddContextReset}
        isCompacting={isCompacting}
        sessionId={activeSessionId}
        hasCustomPrompt={hasCustomPrompt}
        onCustomPromptChange={onCustomPromptChange}
        provider={provider}
        mcp={mcp}
        dialogs={dialogs}
        edit={edit}
        features={features}
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
        backgroundTasks={{
          total: backgroundTaskTotal,
          running: backgroundTaskRunning,
          failed: backgroundTaskFailed,
          onClick: () => {
            viewRouting.setBackgroundTasksSessionId(activeSessionId);
            viewRouting.setBackgroundTasksSidebarOpen(true);
            viewRouting.setInputExpanded(false);
          },
        }}
      />
    </section>
  );
}
