-- ============================================================================
-- 0005_inscript — the fifth layout: Standard Hindi InScript
-- ============================================================================
-- Additive only: seed rows for LayoutId::Inscript and for Hindi Typing Mastery
-- typed on InScript (ProductId::HindiMastery(Inscript)). The product has no
-- Home card and no exercises of its own; it is reached through the layout
-- switch and types the Unicode Hindi course (only passages InScript can type),
-- so its sessions are recorded under their own product and layout instead of
-- being mislabelled as Remington GAIL.
INSERT INTO layouts (id, slug, display_name) VALUES
    (5, 'inscript', 'InScript');

INSERT INTO products (id, slug, display_name, layout_id, is_beginner, display_order) VALUES
    (10, 'hindi-mastery-inscript', 'InScript Typing', 5, 0, 10);
