/// What a reequip part was given: the stack counts, the slot, `slot_changed` and the answer so far.
type Heard = (u32, u32, EquipmentSlot, bool, bool);

/// A reequip part: records what it was given, edits its own copy of the slot, then answers.
struct ReequipProbe(bool, Arc<Mutex<Vec<Heard>>>);

impl ItemPart for ReequipProbe {
    fn item_subscribed(&self) -> &'static [Option<HookId>] {
        &[Some(HookId::ShouldCauseReequipAnimation)]
    }
    fn should_cause_reequip_animation(
        &self,
        _: &dyn ClientHost,
        event: &mut ShouldCauseReequipAnimationEvent<'_>,
    ) {
        assert!(matches!(event.hand, Hand::Main));
        self.1.lock().unwrap().push((
            event.stack.count(),
            event.old.count(),
            event.slot,
            event.slot_changed,
            event.reequip,
        ));
        event.slot = EquipmentSlot::OffHand;
        event.reequip = self.0;
    }
}

#[test]
fn reequip_parts_start_from_vanillas_answer_and_answer_in_turn() {
    install(Vec::<ModEntry>::new().into());
    let heard = Arc::new(Mutex::new(Vec::new()));
    install_parts(Parts {
        items: [true, false]
            .map(|answer| leak(ReequipProbe(answer, heard.clone())) as &dyn ItemPart)
            .into(),
        ..Default::default()
    });
    let stick = |count| ItemStack {
        item: "fixture:stick".into(),
        count,
    };
    let mut reequip = false;
    dispatch_should_cause_reequip_animation(
        &TestHost,
        &stick(2),
        &stick(1),
        Hand::Main,
        EquipmentSlot::MainHand,
        true,
        &mut reequip,
    );
    // The second part is given the first part's answer but the original slot, and its own
    // answer is the last word.
    assert!(!reequip);
    assert_eq!(
        *heard.lock().unwrap(),
        [
            (2, 1, EquipmentSlot::MainHand, true, false),
            (2, 1, EquipmentSlot::MainHand, true, true)
        ]
    );
    install_parts(Parts::default());
}
