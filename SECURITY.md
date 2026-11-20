# Security Policy

## Supported versions

Security fixes land in the latest release of hibi and are not backported.
The earlier TypeScript line, which ended at 0.7.0, receives no fixes.
Upgrade to the latest release before reporting an issue.

## Reporting a vulnerability

Do not open a public issue for a security vulnerability.

Report it privately through GitHub's private vulnerability reporting:
open the repository's **Security** tab and choose **Report a vulnerability**,
or go to https://github.com/npupko/hibi/security/advisories/new.

If you cannot use GitHub's private reporting, email the maintainer at
lazynick7@gmail.com.

Include what you can of:

- a description of the problem and its impact;
- the steps or the repository content that reproduce it;
- the hibi version (`hibi version`), the git version and the operating system;
- a suggested fix.

hibi is maintained by one person on a best-effort basis. Expect an
acknowledgement within a few days. Valid reports are fixed in the next
release, and the disclosure date is agreed with you.

## Threat model

hibi runs inside repositories that other people wrote: a cloned project, a
pull request branch, a dependency checked out next to your own code. The
content of such a repository is untrusted. That covers every tracked file,
including `.hibi/hibi.toml`, the claim records and the documents and code
that claims point at.

**What untrusted repository content must never cause:**

- **Running a command.** hibi never reads a command from the repository. A
  provider command comes only from your user config, and only for a scheme
  that you registered there. Every `git` process hibi starts carries options
  that stop repository content from naming a program for git to run.
- **Writing outside the repository.** hibi writes only inside the
  repository's `.hibi/` directory and to its own cache database in your
  user cache directory. If `.hibi`, `hibi.toml`, a shard directory, a shard
  file or the lock file is a symbolic link, hibi stops before it writes.
- **Sending data off the machine.** Only `triage`, `suggest-evidence` and
  `status --judge-order` contact the network, and only when you have set
  `TYPESAFE_API_KEY`. They never send the text of an ignored file or of a
  sibling repository you did not allow.
- **Reading files you did not offer.** A sibling repository is read only
  when your user config allows it, and only the files that repository
  tracks. A path through a symbolic link is treated as missing.

**What is trusted:**

- Your user config (`HIBI_CONFIG`, or `hibi/config.toml` in your config
  directory). It is the only source of provider commands and of the
  directories of sibling repositories.
- The repository's own `.git/config`, and that of a sibling repository you
  allowed. git writes it locally and a clone does not transfer it. A
  repository copied together with its `.git` directory from an untrusted
  source is outside this model, as it is for git itself.
- The provider programs you register, and the agent or person running hibi.

A way to break any of the rules above with repository content alone is a
vulnerability. Please report it as described above.
