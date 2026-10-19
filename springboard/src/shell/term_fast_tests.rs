    use super::*;

    fn frame(sh: &mut Shell, buf: &mut [u32], t: f64) -> Damage {
        let (w, h) = (sh.w as usize, sh.h as usize);
        let mut s = Surface { w, h, stride: w, px: buf, y0: 0 };
        sh.draw(&mut s, t).clone()
    }

    #[test]
    fn terminal_output_repaints_only_its_rows_and_matches_a_full_frame() {
        let mut sh = tests::shell();
        // a new shell is on the lock screen, which draws its own frame; the terminal frame needs an unlocked person
        tests::unlock(&mut sh);
        sh.open(AppId::Terminal, 1000.0);
        sh.tick(3000.0);
        let mut buf = vec![0u32; 1200 * 1920];
        frame(&mut sh, &mut buf, 3000.0);
        frame(&mut sh, &mut buf, 3100.0);
        assert!(matches!(sh.drawn, Drawn::Term { .. }), "the terminal frame is keyed");
        sh.term.as_mut().unwrap().feed(b"hello");
        let d = frame(&mut sh, &mut buf, 3200.0);
        assert!(!d.is_all() && d.pixels() > 0 && d.pixels() < 1200 * 200, "only the cursor row is sent");
        let mut full = vec![0u32; 1200 * 1920];
        sh.drawn = Drawn::Nothing;
        frame(&mut sh, &mut full, 3200.0);
        assert!(buf == full, "the partial frame equals a full repaint");
    }

    #[test]
    fn settings_scroll_moves_pixels_and_an_unchanged_frame_draws_nothing() {
        // landscape: the Notifications page is taller than the 1200 px screen
        let mut sh = tests::shell();
        sh.set_orientation(crate::gfx::rotate::Orientation::LandscapeLeft, 500.0);
        sh.tick(900.0);
        sh.open(AppId::Settings, 1000.0);
        sh.tick(3000.0);
        // the keyed scroll path is the single column's (the split view draws its pane straight in)
        sh.settings.as_mut().unwrap().allow_split(&sh.ui, false);
        // a page long enough to scroll on the 1920 px screen
        sh.tick(5000.0);
        let mut buf = vec![0u32; 1200 * 1920];
        frame(&mut sh, &mut buf, 5000.0);
        // offsets inside however far the page scrolls (it lists the built-in apps)
        let off_now = |sh: &Shell| sh.settings.as_ref().unwrap().list().unwrap().draw_offset();
        sh.settings.as_mut().unwrap().scroll_to(1e6);
        let max = off_now(&sh);
        assert!(max >= 200.0, "the page must scroll for this test ({max})");
        let mid = (max / 2.0).round();
        sh.settings.as_mut().unwrap().scroll_to(mid);
        frame(&mut sh, &mut buf, 5010.0);
        frame(&mut sh, &mut buf, 5020.0);
        assert!(matches!(sh.drawn, Drawn::Settings { .. }), "the Settings frame is keyed");
        assert!(frame(&mut sh, &mut buf, 5030.0).is_empty(), "nothing changed, nothing drawn");
        assert_eq!(off_now(&sh), mid);
        for off in [mid + 37.0, mid + 20.0, max, (max * 0.2).round()] {
            sh.settings.as_mut().unwrap().scroll_to(off);
            assert_eq!(off_now(&sh), off);
            let d = frame(&mut sh, &mut buf, 5040.0);
            assert!(!d.is_empty(), "scrolling damages the content");
            assert!(matches!(sh.drawn, Drawn::Settings { .. }), "still keyed after a scroll");
            let mut full = vec![0u32; 1200 * 1920];
            sh.drawn = Drawn::Nothing;
            frame(&mut sh, &mut full, 5040.0);
            let bad = buf.iter().zip(&full).filter(|(a, b)| (0..4).any(|c| ((**a >> (c * 8)) & 255).abs_diff((**b >> (c * 8)) & 255) > 6)).count();
            assert!(bad < 1200 * 1920 / 200, "scrolled frame at {off} differs from a full repaint in {bad} px");
            // the keyed state is again the scrolled page for the next step
            frame(&mut sh, &mut buf, 5050.0);
        }
    }
