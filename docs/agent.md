# Agent mode

```bash
prinia -a "which file defines the retry logic, and how many times does it retry?"
prinia -a --write "rename the function parse to parse_config everywhere"
prinia -a --run "run the tests and tell me what fails"
```

With `-a` (`--agent`) the model can look around in the current directory before it answers: it calls tools, `prinia` runs them and returns the results, and this repeats until the model gives a final answer. The answer goes to stdout like any other call, and one line per tool call goes to stderr so you can see what it did.

## What it can do

| Tool | Needs | What it does |
| :--- | :--- | :--- |
| `read_file` | nothing | Reads a text file, with line numbers, optionally a line range |
| `list_dir` | nothing | Lists a directory |
| `search` | nothing | Finds lines containing a text (case-insensitive) under a path; skips hidden directories, `target` and `node_modules` |
| `write_file`, `replace` | `--write` | Creates or overwrites a file; replaces one exact, unique piece of text |
| `run_command` | `--run` | Runs a shell command in the directory and returns its output and exit code; stopped after 60 seconds |

Reading is the default and changes nothing. Writing and running commands are separate switches, so you can allow one without the other. There is no per-action confirmation: the flag is the permission, so use `--write` and `--run` where you would be comfortable letting a script run, and in a git repository where changes can be reviewed.

All file tools stay inside the directory you ran the command in. A path that leaves it, by `..`, an absolute path or a symbolic link, is refused. `run_command` is not limited this way, since a shell command can do anything your account can.

The model sees the contents of every file it reads, so they are sent to the provider like any other prompt.

## Limits and recording

A session stops after 25 tool rounds (`agent_steps`). Tool output is cut at 20000 characters. Caching and `-c` context do not apply; the whole session is saved to the history as one turn in agent mode, with the final answer as its response.

The tools work with Anthropic and OpenAI-compatible providers, but the model has to support tool calling. Small local models often do it poorly.
