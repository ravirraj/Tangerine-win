# Capabilities, operations and execution

This is the shape of the part of the program that decides **what may be done**
with the files that were dragged, and what happens when something is run.

It is written down here because the concepts are easy to mix up, and mixing
them up is how a program ends up offering an action it cannot do.

## The parts, and what each one is

| Part | Where | What it answers |
| --- | --- | --- |
| The selection | `src/radial/selection.rs` | What was dragged, and what kind of thing each item is. |
| The operations | `src/radial/registry.rs` | What the application can do, and what each operation needs. |
| The capabilities | `src/radial/registry.rs` | What the machine can do: which backends are there, and what they can read and write. |
| The eligibility | `src/radial/registry.rs` | Whether one operation may be used for one selection. |
| The menu | `src/radial/actions.rs` | The choices of each of the three wheels. |
| The execution | `src/radial/execution.rs` | What happens when an operation is run, and what comes of it. |

They are kept apart on purpose. A **file category** says what a file is
called. A **capability** says what a backend can do. An **operation** says
what the program offers. **Eligibility** says whether an operation fits a
selection. None of them says that anything works.

## The selection

`Selection::inspect` reads the attributes of each item and nothing else. No
file is opened, no folder is looked inside, and no processor is run, because
the menu is built while the user is dragging and the wheel cannot wait.

Each item is told:

- its **kind** — image, video, audio, pdf, document, spreadsheet, presentation,
  archive, text, folder, or unknown;
- its **format** — the name the file is called by, in lower case and without
  the dot: `png`, `jpeg`. A file with no name that says anything has no format;
- whether it could be **read** at all.

The kind and the format are both worked out from the name of the file, so
both are guesses. They are good enough for a menu, and no more than that.

An item that cannot be read stays in the selection and is counted as
unreadable. It is not quietly left out, and the rest of the selection is never
treated as the whole of it.

## The operations

An operation says:

- its **name**, which is its identity — `convert.png`. The label may be
  translated and the picture may change; the name may not;
- the **wheel** it belongs to — the three wheels are fixed;
- the **kinds of file** it works on;
- its **batch** — how many items it takes, and whether they have to be alike;
- whether it works on a **whole folder**;
- its **output** — where the result goes and what it is written in;
- what to do when the result is **already there**;
- whether it is **implemented**;
- what it **requires** — nothing, or a processor by name;
- whether it **changes the files it is given**.

The last of those is worked out from the output: an operation that writes over
the original changes the user's files, and one that writes a new file beside it
does not.

## The capabilities

A **processor** is a backend: the thing that reads a file and writes a file.
It says:

- its **name** — what an operation asks for;
- the formats it can **read**;
- the formats it can **write**.

The **environment** is the set of processors that are there. The registry says
what an operation needs; the environment says what is here. An operation that
needs a processor cannot run until that processor is in the environment.

Some operations need nothing at all. Those are the **universal file actions**
— copy the path, make a duplicate, rename, reveal in Explorer — and they work
on any file, with no conversion backend behind them. They are marked by
`Requirement::None`, and the eligibility rules let them through without
looking for a processor.

## The eligibility

There is **one** path from a selection and an environment to the operations on
the wheel, and it is `registry::eligibility`. The menu is built from it
(`registry::menu_from`), and the execution checks against it
(`execution::run`). The rules, in the order the reasons are given:

1. there is something selected, and every item of it could be read;
2. the operation takes as many items as the selection has — one, several, or
   several that have to be alike — and the items are of a kind it works on,
   and a folder only when it works on folders;
3. the operation is implemented, and the processor it needs is there;
4. the processor can read every format that was dragged, and can write the
   format the operation is to produce.

The rules look at the **whole** selection. An operation is offered when every
selected item is one it can work on, not when one of them happens to be. An
image and a PDF together are not an image selection with a PDF in it.

The reason is always one of `Ineligible`, and it is safe to put in the log: it
never carries a path.

## The execution

`execution::run` takes a **request** — the name of an operation and the files
it is to work on — and gives back an **outcome**:

- `Succeeded` with the files it wrote;
- `Failed` with the reason;
- `Cancelled`, when the user stopped it.

The request is checked before anything is run. The operation has to exist, the
files it names have to be the files that were dragged, and the operation has
to be one that may be used for them. What happens then is up to the handler
for the operation.

**There is no handler yet.** Every operation is refused, and the refusal says
why: *not implemented*. Nothing pretends to have worked, and no success is
reported that did not happen.

The reasons are `Failure`, and they are safe to put in the log: a failure
never carries a path either, so the log says what happened and not which file
it happened to.

## What is not here yet

- No handler. Every operation is refused, and the registry says so.
- No processor. The environment is empty, so nothing that needs one is
  offered.
- No batch work, no progress and no cancelling of work in progress. The
  `Cancelled` outcome is part of the contract, and nothing produces it yet.
- The formats are worked out from the name of the file, not from what is in
  it. A file called `photo.png` may hold anything at all.

## The menu

The three wheels are fixed. What is on them is not: the wheel shows the
operations that may really be done with the files that were dragged, and a
wheel with nothing to offer is empty. That is what a selection that nothing
can be done with looks like, and the application says so in the log.

The **development wheel** (`Ctrl+Alt+T`) is a look at the design, and it
shows every operation there is, whether or not it can be run. It is marked on
screen and in the log, and nothing it reports is a real Explorer drag.
