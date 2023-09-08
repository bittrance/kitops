# kitops - generic GitOps agent

kitops monitors one or more Git repositories and performs arbitrary actions when those repositories are updated.

kitops is under development and not yet ready to be used.

## Roadmap

The plan forward, roughly in falling priority:

- [ ] --poll-once to check all repos that are due, then exit
- [ ] verify azdo support
- [ ] allow configuring notification actions
- [ ] specialized notification action to update github status
- [ ] new git sha and branch name in action env vars
- [ ] docker packaging
- [ ] readme with design and deployment options
- [ ] intelligent gitconfig handling
- [ ] allow git commands in workdir (but note that this means two tasks can no longer point to the same repo without additional changeas)
- [ ] useful logging (log level, json)
- [ ] support Amazon S3 as state store
- [ ] support Azure Blob storage as state store
