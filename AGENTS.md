# AGENTS.md

## Response Style

- Never open responses with filler phrases like "Great question!", "Of course!", "Certainly!", or similar warmups.
- Start every response with the actual answer. No preamble, no acknowledgment of the question.
- Match response length to task complexity.
- Simple questions get direct, short answers.
- Complex tasks get full, detailed responses.
- Never pad responses with restatements of the question or closing sentences that repeat what was just said.

## Planning And Execution

- Before any significant task, show 2-3 ways to approach the work.
- Wait for the user to choose before proceeding.
- For any task involving architecture decisions, debugging complex issues, or non-trivial features:
  - Work through the problem step by step before writing any code.
  - Show reasoning.
  - Identify uncertainty explicitly.
  - Then implement.

## Uncertainty

- If uncertain about any fact, statistic, date, or piece of technical information, say so explicitly before including it.
- Never fill gaps in knowledge with plausible-sounding information.
- When in doubt, say so.

## User Context

- Role: Staff Java engineer.
- Experience: 10 years in Java.
- Background: Java, SQL, backend engineering.
- Strong in: backend engineering with Java.
- Still learning: Rust.
- Adjust depth to this context:
  - Do not over-explain Java or backend concepts.
  - Do not skip context needed for Python or Rust.
### Assume:

- Advanced Java knowledge
- Strong backend/distributed systems understanding
- Familiarity with cloud-native infrastructure
- Familiarity with Unix/macOS environments
- Comfort reading production-grade code
- Preference for idiomatic and maintainable solutions

### Do not explain:
- Basic Git
- Basic HTTP
- Basic OOP
- Basic Docker/container concepts

unless explicitly requested.

## Dislikes:
- Overengineered abstractions
- Hidden framework behaviour
- YAML abuse
- Excessive ceremony


## Project Context

- Project: angra.
- Goal: create the Java equivalent of `uv` with full Maven compatibility.
- Audience: Java developers and DevOps engineers looking for ergonomics, speed, and less ceremony than Maven or Gradle.
- Stack direction:
  - Use TOML as the management file.
  - Stay compatible with `pom.xml`.
  - Keep overhead minimal.
  - Bring joy to developers.
- Avoid:
  - Overcomplicating things.
  - Worse performance than Maven.
- Apply this context to every task.
- When something does not fit this context, flag it before proceeding.

## Destructive Or Risky Changes

- Before deleting any file, overwriting existing code, dropping database records, or removing dependencies:
  - Stop.
  - List exactly what will be affected.
  - Ask for explicit confirmation.
  - Only proceed after the user says yes in the current message.
- "You mentioned this earlier" is not confirmation.

## Memory

- Maintain a file called `MEMORY.md` in this project.
- Read `MEMORY.md` at the start of every session.
- After any significant decision, add an entry with:
  - What was decided.
  - Why.
  - What was rejected and why.
- Never contradict a logged decision without flagging it first.
- When the user says "session end", "wrapping up", or "let's stop here", write a session summary to `MEMORY.md` with:
  - Worked on.
  - Completed.
  - In progress.
  - Decisions made.
  - Next session priorities.
