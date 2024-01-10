# kitops - generic GitOps agent

kitops monitors one or more Git repositories and performs arbitrary actions when those repositories are updated. It can be used by devops:y people to implement a wide variety of Continuous Delivery and Deployment scenarios and by systems and infra teams to provide GitOps-style workflows to the developers they support. [gitops.tech](https://www.gitops.tech/):

> GitOps is a way of implementing Continuous Deployment for cloud native applications. It focuses on a developer-centric experience when operating infrastructure, by using tools developers are already familiar with, including Git and Continuous Deployment tools. The core idea of GitOps is having a Git repository that always contains declarative descriptions of the infrastructure currently desired in the production environment and an automated process to make the production environment match the described state in the repository. If you want to deploy a new application or update an existing one, you only need to update the repository[.]

It turns out that this model can easily be applied to any configuration management task, by leveraging the fact that Git can be used to version any kind of text. kitops tries to fill this role.

**kitops is under development and not yet ready to be used.**

## Getting started

The simplest way to test kitops is to run the Docker image:

```shell
docker run bittrance/kitops --url https://github.com/bittrance/kitops --action 'echo "kitops was updated"'
```

## Rationale

### Why would you want to use kitops?

The traditional model is to have your pipelines (or actions or workflows) push deployments onto target environments.

![Push-style continuous deployment](images/cd-push-model.png)

This model has several potential weaknesses:

- The pipeline has to know where the result should be delivered and deployed
- When the number of target environments grow numerous, some may fail while others pass, making it hard to get a passing deploy
- It requires the pipeline (typically executing outside the target environment) to have extensive permissions
- Called APIs have to be accessible over the Internet (or require a VPN or similar)
- It is hard to delegate responsibility for the target environment to a third party

kitops enables the environment to "pull" deployments from a git repository.

![Pull-style continuous deployment](images/cd-pull-model.png)

This model:

- is scalable - only repository rate limiting caps the number of target environments
- adheres to the Principle of Least Privilege - the pipeline has no access to the environment and the environment only needs read access to the repository. This is particularly relevant in Azure, where granting permissions to a pipeline requires extensive AAD permissions, but creating a service principal for kitops can be delegated to developers via the `Application Developer` role.
- is NAT-friendly - the environment only needs to be able to make outbound connections to the git server
- allows a third party to take responsibility for the target environment

### How does kitops work?

kitops is a statically compiled binary. It uses only pure Rust libraries and it therefore depends only on libc (and an ssh binary where you want git+ssh support). It supports a wide variety of platforms and can be deployed in many different ways:

- as a long-running process on a VM
- as a periodic job
- as a long-running container
- as a CLI tool to perform a single run

kitops runs each task on a schedule.

<picture>

Each time kitops successfully applies all the actions of a task, it updates its state file. The state file acts as memory between executions so if kitops is run as a periodic job, you should point --state-file to persistent file storage.

kitops will clone repositories that are not already present in --repo-dir so you can use ephemeral storage for this, but if your repositories are large, you may want to keep repositories on persistent storage too. This allows fetching only new changes, dramatically reducing network traffic.

## Example use cases

### Infrastructure-as-code continuous deployment

Use a serverless platform such as AWS Fargate or Azure Container Apps to run kitops as a periodic container job that applies infrastructure changes as they occur. Becuase kitops only takes a second to start and check for changes, this solution will typically cost a few dollars per month to run.

This scenario still requires , manually maintained infrastructure definition for kitops itself as it does not currently have special support to update itself.

### Roll your own container-based build servers

One use case for Kitops is to combine it with [act](...). Kitops pullsrepo  changes and simply inokes act which will give you a local runner. Act will interact with the loval Docker daemon, much like you were using GiHub-hosted runners.

This use case requires a virtual machine, because there is currently no container orchestration platform that gives access to the local Docker socket, as required by `act`.

kitops cannot yet coordinate execution across multiple nodes, so you will have to balance your repositories across build servers manually.

## Alternatives

- [snare](https://tratt.net/laurie/src/snare/) - tool with similar design goals, but limited to GitHub webhooks (i.e. push-based).

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
- [x] readme with design and deployment options
- [ ] release binaries for major platforms
- [ ] branch patterns allows a task to react to changes on many branches
- [ ] Support GitHub runner long polling interface
- [ ] intelligent gitconfig handling
- [ ] allow git commands in workdir (but note that this means two tasks can no longer point to the same repo without additional changeas)
- [ ] useful logging (log level, json)
- [ ] lock state so that many kitops instances can collaborate
- [ ] support Amazon S3 as state store
- [ ] support Azure Blob storage as state store
- [x] GitHub app for checking out private repo
