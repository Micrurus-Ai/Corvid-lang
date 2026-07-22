// Sign-in buttons (slice 51p). Renders one button per identity provider
// and redirects to `/auth/{provider}/login` through the client. Feed it
// the provider names from the contract's identity block.

import { type ReactNode } from "react";
import { CorvidClient } from "@corvid/client";

export interface CorvidSignInProps {
  client: CorvidClient;
  /** Provider wire-names (e.g. from `contract.identities[0].providers`). */
  providers: string[];
  /** Render a provider's label. Defaults to "Sign in with {provider}". */
  labelFor?: (provider: string) => string;
  className?: string;
}

/** A row of provider sign-in buttons. */
export function CorvidSignIn(props: CorvidSignInProps): ReactNode {
  const { client, providers, labelFor, className } = props;
  return (
    <div className={className} style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
      {providers.map((p) => (
        <button key={p} className="corvid-signin" data-provider={p} onClick={() => client.login(p)}>
          {labelFor ? labelFor(p) : `Sign in with ${p}`}
        </button>
      ))}
    </div>
  );
}
