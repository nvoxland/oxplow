import type { AgentStatus, Batch, BatchWorkState } from "../../api.js";
import { BatchQueueSection } from "../LeftPanel/BatchQueueSection.js";

interface Props {
  batches: Batch[];
  batchWorkStates: Record<string, BatchWorkState>;
  agentStatuses: Record<string, AgentStatus>;
  selectedBatchId: string | null;
  activeBatchId: string | null;
  onSelectBatch(batchId: string): Promise<void>;
  onCreateBatch(title: string): Promise<void>;
  onReorderBatch(batchId: string, targetIndex: number): Promise<void>;
  onPromoteBatch(batchId: string): Promise<void>;
  onCompleteBatch(batchId: string): Promise<void>;
}

export function BatchesPanel(props: Props) {
  return (
    <div style={{ flex: 1, overflow: "auto", padding: 12, fontSize: 12 }}>
      <BatchQueueSection {...props} />
    </div>
  );
}
