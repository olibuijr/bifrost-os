//! Parity sdk-rest-09: headless tests for the layer session that apps submit their trees through. No test composes a frame
//! or touches a compositor; `presentation` returns the layers a renderer would draw, and the tests read that list.

use bifrost_layers::session::{LayerSession, SessionError};
use bifrost_layers::tree::ApplyError;
use bifrost_layers::{Change, LayerId};

#[test]
fn a_new_session_numbers_its_first_transaction_one() {
    let session = LayerSession::new();
    assert_eq!(session.next_seq(), 1);
    assert_eq!(session.transaction().seq, 1);
    assert!(session.tree().layers.is_empty());
}

#[test]
fn a_commit_advances_the_sequence_and_returns_it() {
    let mut session = LayerSession::new();
    let mut tx = session.transaction();
    tx.push(Change::Create(LayerId(1))).push(Change::SetRoot(LayerId(1)));
    assert_eq!(session.commit(&tx, 0.0), Ok(1));
    assert_eq!(session.next_seq(), 2);
    assert_eq!(session.tree().root, Some(LayerId(1)));
}

#[test]
fn a_repeated_sequence_number_is_refused_and_changes_nothing() {
    let mut session = LayerSession::new();
    let mut first = session.transaction();
    first.push(Change::Create(LayerId(1))).push(Change::SetRoot(LayerId(1)));
    session.commit(&first, 0.0).unwrap();

    let mut replay = bifrost_layers::Transaction::new(1);
    replay.push(Change::Create(LayerId(2)));
    assert_eq!(session.commit(&replay, 0.0), Err(SessionError::Stale { last: 1, got: 1 }));
    assert_eq!(session.tree().layers.len(), 1, "the refused batch created no layer");
}

#[test]
fn an_older_sequence_number_is_refused_after_a_newer_commit() {
    let mut session = LayerSession::new();
    session.commit(&bifrost_layers::Transaction::new(1), 0.0).unwrap();
    session.commit(&bifrost_layers::Transaction::new(2), 0.0).unwrap();
    assert_eq!(
        session.commit(&bifrost_layers::Transaction::new(1), 0.0),
        Err(SessionError::Stale { last: 2, got: 1 })
    );
    assert_eq!(session.next_seq(), 3);
}

#[test]
fn a_zero_sequence_number_is_refused_on_a_new_session() {
    let mut session = LayerSession::new();
    assert_eq!(
        session.commit(&bifrost_layers::Transaction::new(0), 0.0),
        Err(SessionError::Stale { last: 0, got: 0 })
    );
}

#[test]
fn a_refused_change_does_not_consume_its_sequence_number() {
    let mut session = LayerSession::new();
    let mut bad = session.transaction();
    bad.push(Change::Create(LayerId(0)));
    assert_eq!(session.commit(&bad, 0.0), Err(SessionError::Apply(ApplyError::ZeroId)));
    assert_eq!(session.next_seq(), 1, "a refused commit leaves the next sequence number free");

    let mut good = session.transaction();
    good.push(Change::Create(LayerId(3))).push(Change::SetRoot(LayerId(3)));
    assert_eq!(session.commit(&good, 0.0), Ok(1));
}

#[test]
fn presentation_draws_the_root_and_its_visible_sublayers() {
    let mut session = LayerSession::new();
    let mut tx = session.transaction();
    tx.push(Change::Create(LayerId(1)))
        .push(Change::SetRoot(LayerId(1)))
        .push(Change::Create(LayerId(2)))
        .push(Change::Insert { parent: LayerId(1), child: LayerId(2), index: 0 });
    session.commit(&tx, 0.0).unwrap();
    assert_eq!(session.presentation(0.0).len(), 2);

    let mut hide = session.transaction();
    hide.push(Change::SetHidden(LayerId(2), true));
    session.commit(&hide, 0.0).unwrap();
    assert_eq!(session.presentation(0.0).len(), 1, "a hidden sublayer is left out of the frame");
}
