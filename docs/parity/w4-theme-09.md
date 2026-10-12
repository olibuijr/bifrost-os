# Wave 4, theme-09: Distributed actors

## What was built

`springboard/sdk/src/post_delivery.rs` (new): a pure model of what happens to a cross-app broadcast post when its
receiver is suspended or not running. It mirrors the official `DistributedNotificationCenter.SuspensionBehavior`
cases:

- `drop`: the post is lost while the receiver is suspended.
- `coalesce`: only the latest post of each name is queued.
- `hold`: posts are queued in arrival order, up to `MAX_HELD` (32). When the queue is full, the oldest post is
  discarded. The official page says the queue size is set by the server and gives no number, so 32 is a BifrOSt choice.
- `deliverImmediately`: a suspended receiver gets the post at once. A receiver that is not running has nothing to
  reach, so the post is dropped.

`PostQueue::send` decides the outcome per receiver (`Delivered`, `Held`, `Dropped`). `PostQueue::resume` returns what
was held when the receiver runs again. `Post::new` enforces the existing cross-app limits: at most 32 user-info
entries and 1024 characters per value. An empty name is refused.

Treating a receiver that is not running like a suspended one is an assumption. The official pages describe the
suspension rules without covering a process that does not exist. The model applies the same rules and records the
assumption in its module comment.

`pub mod post_delivery;` was added to `springboard/sdk/src/lib.rs`.

Not wired: nothing calls `PostQueue` yet. The shell still relays posts to running apps only
(`springboard/sdk/src/notification.rs`, `ToShell::DistPost`). Wiring it into the shell and testing on the VM or Midget
is follow-up work.

## Official API names mirrored

- `DistributedNotificationCenter.SuspensionBehavior` (`drop`, `coalesce`, `hold`, `deliverImmediately`)
  https://developer.apple.com/documentation/foundation/distributednotificationcenter/suspensionbehavior

## REA evidence

- `ev_4bce1130b53a600319984bb52c8f6a81f0ed29f32735eef7cdcb29350c3148be` (the suspension behaviour page, rendered)

## Matrix rows closed

| Framework slug | Feature (as written in data) | New status | BifrOSt path |
|---|---|---|---|
| darwin-notify | Posting to a non-running app (suspensionBehavior) | partial | springboard/sdk/src/post_delivery.rs |

## Rows reviewed and left unchanged

- Swift "Distributed actors" (gap): `springboard/sdk/src/distributed.rs` already holds the call side from wave 1. Remote
  delivery between processes is still missing, so the row stays a gap.
- Darwin Notify "Signal-based delivery" and "Posting a named notification": no change.
- The other theme-09 rows (user notifications, network extension, CallKit, EventKit, and the rest) have no pure-Rust
  model in this round.

## Tests

`cargo` was not run, per the task rules. The module has seven `#[cfg(test)]` tests, with expected values worked out by
hand from the rules above:
`a_running_receiver_gets_every_post_whatever_the_behaviour`, `drop_loses_the_post_while_suspended`,
`coalesce_keeps_only_the_latest_post_of_each_name`, `hold_keeps_arrival_order_and_discards_the_oldest_when_full`,
`deliver_immediately_reaches_only_a_running_process`, `a_stopped_receiver_holds_posts_until_it_runs_again`,
`post_limits_are_checked_before_queueing`.

`python3 tools/check-names.py` was run in the worktree and printed no hits.
