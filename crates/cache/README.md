# grid-cache

Single responsibility: verification verdicts keyed by what the verdict actually
depended on.

A key is frozen at record time from a per-kind closure over the chart:

| kind | key covers |
|---|---|
| `lint` | the datum alone (`span_hash`) — lint reads one span |
| `typecheck` | the datum plus its transitive **proven** out-edge closure |
| `test` | the datum plus its transitive full out-edge closure |

Soundness is one-directional (claim C4): a key that cannot be fully established
— unknown datum, or closure past fuel — is **rejected at record time**, never
stored, and never answers a check. A hit therefore implies an established
closure; a miss implies nothing except "run it".
