# kitops - generic GitOps agent

kitops monitors one or more Git repositories and performs arbitrary actions when those repositories are updated.

kitops is under development and not yet ready to be used.

## Roadmap

The plan forward, roughly in falling priority:

- [x] --poll-once to check all repos that are due, then exit
- [ ] verify azdo support - Byron/gitoxide#1025
- [x] Reasonable timeout duration entry (i.e. not serde default secs/nanos)
- [x] Errors in scoped blocks should cancel, not wait for watchdog to reach deadline
- [x] allow configuring notification actions
- [x] proper options validation (e.g. config-file xor url/action)
- [x] specialized notification action to update github status
- [x] new git sha and branch name in action env vars
- [x] changed task config should override state loaded from disk
- [x] docker packaging
- [ ] readme with design and deployment options
- [ ] intelligent gitconfig handling
- [ ] allow git commands in workdir (but note that this means two tasks can no longer point to the same repo without additional changeas)
- [ ] useful logging (log level, json)
- [ ] lock state so that many kitops instances can collaborate
- [ ] support Amazon S3 as state store
- [ ] support Azure Blob storage as state store
- [x] GitHub app for checking out private repo
