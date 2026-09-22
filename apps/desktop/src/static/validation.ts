import { call } from '../../../../crates/cdna-browser/runtime';
import type { Proposal } from '../api';
export type RankRequest = Pick<Proposal, 'schema_version' | 'request_id' | 'workspace_id' | 'domain' | 'context' | 'candidates'> & { mode: 'imitate' | 'advisor'; allow_cloud: boolean; include_evidence: boolean };
export async function validateProposal(value: unknown): Promise<Proposal> {
  const proposal = await call('validate_proposal', value) as Proposal;
  if (proposal.case_kind !== 'hypothetical') throw new Error('公開体験では架空のケースのみ入力してください。 [INPUT_INVALID]');
  return proposal;
}
export async function validateRank(value: unknown): Promise<RankRequest> {
  const request = await call('validate_rank', value) as RankRequest;
  if (request.allow_cloud) throw new Error('公開体験はブラウザ内でのみ実行します。 [CONSENT_REQUIRED]');
  return request;
}
