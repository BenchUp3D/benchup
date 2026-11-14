# Copilot security review

The repository-wide review guidance lives in
[`../copilot-instructions.md`](../copilot-instructions.md). It asks Copilot to
report evidence-backed security and malicious injection findings directly on PRs.

## Activate on GitHub

Committing a ruleset JSON file does **not** activate it. A repository administrator
must import [`copilot-security-review.json`](copilot-security-review.json):

1. Open the repository's **Settings → Rules → Rulesets**.
2. Select **New ruleset → Import a ruleset** and upload the JSON file.
3. Confirm the ruleset is **Active**, targets the default branch, and enables
   **Automatically request Copilot code review**, **Review new pushes**, and
   **Review draft pull requests**. Save the imported ruleset.
4. Under **Settings → Copilot → Code review**, ensure **Use custom instructions
   when reviewing pull requests** is enabled.
5. Open an eligible PR containing the instruction file, or request Copilot as a
   reviewer on an existing PR. Confirm it posts a review; push another commit to
   confirm automatic re-review. Keep the instruction file in subsequent branches.

The ruleset only adds automatic review requests; it preserves other repository
rules and does not configure auto-approval or a required security status check.
Automatic reviews require eligible Copilot access and available usage allowance.
They do not guarantee coverage of every contributor or every file type. Use
required scanner checks and human review for merge enforcement.

Copilot currently reads instructions from the PR's head branch, so this file is
review guidance, not a tamper-proof security boundary. Have a maintainer examine
changes to review instructions and security configuration independently.

References: [automatic review configuration](https://docs.github.com/en/copilot/how-tos/copilot-on-github/set-up-copilot/configure-code-review),
[custom review instructions](https://docs.github.com/en/copilot/how-tos/copilot-on-github/customize-copilot/add-custom-instructions/add-repository-instructions),
and [ruleset import](https://docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/managing-rulesets/managing-rulesets-for-a-repository).
