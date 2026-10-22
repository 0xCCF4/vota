# Vota

A small web app where an admin issues personal Ballots, each reachable by its own URL, and a single voter answers it through a web UI or API.

## Language

**Ballot**:
A single voter's personal voting form, reachable by its own unguessable URL. It carries the question, options, deadlines, style and Voter Reference. Exactly one voter per Ballot.
_Avoid_: Pool, poll (a poll implies many voters)

**Ballot Token**:
The unguessable secret in a Ballot's URL. Whoever holds it acts as the Ballot's voter; it is the voter's only credential.
_Avoid_: Ballot ID, poll URI, link code

**Voter Reference**:
A hidden identification string that the admin attaches to a Ballot to know who it belongs to. It is never shown to the voter, and is unique within a Group.
_Avoid_: Hidden ID, identification field

**Group**:
An optional free-text label the admin attaches to several Ballots so their Responses can be fetched together, e.g. all Ballots of one survey. Ballots in a Group stay independent.
_Avoid_: Survey, campaign, batch

**Theme**:
A named look for the voting screen, either built into the app or loaded from the server's theme folder. Each Ballot uses exactly one Theme.
_Avoid_: Style, template, skin

**Theme Tweak**:
A small per-Ballot adjustment on top of its Theme, such as an accent colour or logo.
_Avoid_: Custom style, override

**Voting Deadline**:
The moment from which a Ballot no longer accepts or changes its Response. The admin can extend it, which reopens a closed Ballot.
_Avoid_: Expiry, expiry date

**Purge Date**:
The moment from which the whole Ballot, including its Response and Voter Reference, is gone: it is no longer served from then on, and the purge sweep deletes it from the server. It is never before the Voting Deadline.
_Avoid_: Expiry, data expiry, retention date

**Response**:
The answers a voter has submitted on a Ballot. On a Ballot that has Allow No Option on, a Response may select no Option at all; it is still a Response (the Ballot is Answered). "No Response" always means the voter has not submitted one.
_Avoid_: Vote, submission, answer set, empty response (it reads like "no Response"; say "a Response of no Option")

**Option**:
One of the possible answers listed on a Ballot.
_Avoid_: Answer, choice

**Choice Mode**:
Whether a Ballot is *single choice* (exactly one Option) or *multiple choice* (one or more Options). A Fallback is never empty; a Response neither, unless the Ballot has Allow No Option on.
_Avoid_: Answer type, question type

**Allow No Option**:
A setting of a multiple-choice Ballot, off by default: the voter may answer with no Option at all ("none of these"). Such a Response makes the Ballot Answered, so the Fallback does not apply. A single-choice Ballot can't have it on. Like any setting, the admin can change it only while the Ballot is Unanswered.
_Avoid_: Allow empty response, allow none, optional answer

**Submission Mode**:
Whether a Ballot's Response is *Final* (submitted once, then locked) or *Editable* (can be changed until the Voting Deadline).
_Avoid_: Edit mode, single submit

**Fallback**:
An optional set of Options, chosen by the admin, that is recorded as the Ballot's result if the voter has not responded by the Voting Deadline. It obeys the Ballot's Choice Mode and may only contain the Ballot's own Options.
_Avoid_: Fallback option, default answer, default option

**Ballot Status**:
Where a Ballot stands: **Unanswered** (no Response yet, or the deadline passed without one and there is no Fallback), **Answered** (the voter submitted a Response), or **Auto-fallback** (the deadline passed without a Response and the Fallback was applied).
_Avoid_: State, closed, expired

**Admin Token**:
A secret credential that grants access to the admin API. It is created on the server by the operator with the CLI, may carry a label and an expiry, can be revoked, and is stored only as a hash. The operator may instead provision one in a file the server reads at startup (an Admin Token from a file): no label, expiry or ID, revoked by removing the file from the configuration.
_Avoid_: API key, admin secret, password
