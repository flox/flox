from scripts.pr_analysis.lib.reviewers import classify, seed_rows, TIER1, TIER2


def tier1_reviewers_get_weight_three():
    for login in TIER1:
        r = classify(login, "User")
        assert r.tier == 1
        assert r.weight == 3.0


def tier2_reviewers_get_weight_two():
    for login in TIER2:
        r = classify(login, "User")
        assert r.tier == 2
        assert r.weight == 2.0


def unknown_human_gets_weight_one():
    r = classify("random_contributor", "User")
    assert r.tier == 3
    assert r.weight == 1.0


def bot_gets_weight_zero_regardless_of_login():
    r = classify("ysndr", "Bot")
    assert r.tier == 4
    assert r.weight == 0.0


def seed_rows_covers_both_tiers():
    rows = seed_rows()
    logins = {row[0] for row in rows}
    assert set(TIER1) | set(TIER2) <= logins
    assert len(rows) == len(TIER1) + len(TIER2)
