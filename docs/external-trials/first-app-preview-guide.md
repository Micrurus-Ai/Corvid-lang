# Build your first app in Corvid

Pick a small app you actually want. Before you open the docs, write one sentence describing what a person should be able to do with it. If you need an idea, build a watchlist where someone can add a film and mark it watched. Your own idea is welcome.

Allow about 45 minutes. Use your usual editor and tools. You can stop at the first blocker and still give us valuable feedback.

## Get the preview

Sign in to GitHub and download the artifact for your computer. The download contains a platform archive. Extract that archive too, and keep its folders together. Check the runtime prerequisite for your platform, especially if the executable fails to start. [Runtime prerequisites](../book/01-install.md#candidate-runtime-prerequisites)

| Platform | x64 | ARM64 |
| --- | --- | --- |
| Windows | [Download](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45029097743/artifacts/14757268719) | [Download](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45029097743/artifacts/14757273191) |
| macOS | [Intel](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45029097743/artifacts/14757567090) | [Apple silicon](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45029097743/artifacts/14757327358) |
| Linux | [Download](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45029097743/artifacts/14757671043) | [Download](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45029097743/artifacts/14757695364) |

Open a terminal in the extracted directory that contains `bin`. In PowerShell, run `$env:Path = (Resolve-Path ./bin).Path + ";" + $env:Path`. In a macOS or Linux shell, run `export PATH="$PWD/bin:$PATH"`. This selects the preview for this terminal session. If you use VS Code, **Extensions: Install from VSIX** installs the bundled `extensions/corvid.vsix`.

```sh
corvid --version
corvid new first-app
cd first-app
corvid check
corvid run
```

Check that the reported revision starts with `8a0c416`. Open `src/main.cor` and build one interaction from your sentence. For a browser app, the browser guide shows how to create and serve a page. If an AI feature is part of your idea, try it and tell us what setup it needed. [Quickstart](../book/02-quickstart.md) · [Browser guide](../book/19-browser-views.md) · [Agents guide](../book/10-agents.md)

## Tell us what happened

Reply privately to the person who invited you. A few lines are enough.

1. What did you set out to build, and what could you actually do when you stopped?
2. Roughly how long did it take to get your first interaction working? If you stopped earlier, where did you stop?
3. At the first confusing moment, what did you write or try? What did you expect, and what happened? A short snippet or exact error is welcome.
4. Which docs, people, or AI tools helped? What would you change first?

If you are happy for us to ask in 30 days what you chose for your next feature, add “follow-up welcome.” Sharing source code is optional.

## At the 30-day follow-up

If you opted in, a short private reply about what happened since the trial is enough.

1. What was the next feature or project you worked on? Which stack did you actually use, and why? If you had no new work, say so.
2. Did a missing capability or a difficult step affect that choice? What were you trying to do, and how did you get past it, switch stacks or stop?
3. Did you recommend Corvid to anyone on your own initiative? You can describe the recommendation without naming the person. With their consent, we can also record whether they installed Corvid and what they built independently.
