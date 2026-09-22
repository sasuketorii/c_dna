import type { Proposal } from "./api";

/** Session-only exposure history; never mutates previously saved observations. */
export class ExposureHistory {
  private readonly families = new Set<string>();
  private readonly records = new Set<string>();

  mark(proposal: Proposal, recordId?: string | null): void {
    this.families.add(this.key(proposal.workspace_id, proposal.family_id));
    if (recordId) this.records.add(this.key(proposal.workspace_id, recordId));
  }

  apply(proposal: Proposal, recordId?: string | null): Proposal {
    return {
      ...proposal,
      model_exposure:
        proposal.model_exposure ||
        this.families.has(this.key(proposal.workspace_id, proposal.family_id)) ||
        (!!recordId && this.records.has(this.key(proposal.workspace_id, recordId))),
    };
  }

  clear(): void {
    this.families.clear();
    this.records.clear();
  }

  private key(workspaceId: string, id: string): string {
    return `${workspaceId}:${id}`;
  }
}
