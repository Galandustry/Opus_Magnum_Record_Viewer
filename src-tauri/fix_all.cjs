// fix_all.cjs — single pass, idempotent
const fs = require('fs');
let c = fs.readFileSync('src/lib.rs', 'utf8');

// === 1. OmScoreDTO: add #[serde(default)] to 6 fields ===
const fields = ['cost','cycles','area','instructions','overlap','trackless'];
for (const f of fields) {
  // Only add if not already present
  const pat = new RegExp(`(?<!#\\[serde\\(default\\)\\]\\n)    pub ${f}: (i32|bool),`);
  const repl = `    #[serde(default)]\n    pub ${f}: $1,`;
  if (!c.includes(`#[serde(default)]\n    pub ${f}:`)) {
    c = c.replace(`    pub ${f}: i32,`, `    #[serde(default)]\n    pub ${f}: i32,`);
    c = c.replace(`    pub ${f}: bool,`, `    #[serde(default)]\n    pub ${f}: bool,`);
  }
}

// === 2. Add INF fields to OmScoreDTO ===
if (!c.includes('area_inf_level')) {
  c = c.replace(
    '    pub rate: Option<f64>\n}',
    '    pub rate: Option<f64>,\n    pub area_inf_level: Option<i32>,\n    pub area_inf_value: Option<f64>,\n    pub height_inf: Option<f64>,\n    pub width_inf: Option<f64>,\n    #[serde(rename = "boundingHexINF")]\n    pub bounding_hex_inf: Option<f64>,\n}'
  );
}

// === 3. OmRecordDTO: puzzle optional + smart_formatted_categories ===
if (!c.includes('pub puzzle: Option<OmPuzzleDTO>')) {
  c = c.replace('    pub puzzle: OmPuzzleDTO,', '    #[serde(default)]\n    pub puzzle: Option<OmPuzzleDTO>,');
}
if (!c.includes('pub smart_formatted_categories')) {
  c = c.replace(
    '    pub category_ids: Option<Vec<String>>,',
    '    pub category_ids: Option<Vec<String>>, \n    pub smart_formatted_categories: Option<String>,'
  );
}

// === 4. Replace .puzzle.id accesses ===
c = c.replace(/\.puzzle\.id\s*==\s*puzzle_id/g, '.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)');

// === 5. Replace parse block with elastic parsing ===
const oldBlock = `    if let Some(body) = saved_body {
        match serde_json::from_str::<Vec<OmRecordDTO>>(&body) {
            Ok(remote_records) => {
                println!("[ZLBB_PARSER]: {} records from API.", remote_records.len());
                {
                    let memory_state = app_clone.state::<MemoryState>();
                    let mut vault = memory_state.record_vault.lock().unwrap();
                    for remote in &remote_records {
                        let exists = vault.iter().any(|local|
                            local.puzzle.id == remote.puzzle.id && local.full_formatted_score == remote.full_formatted_score
                        );
                        if !exists {
                            vault.push(remote.clone());
                        }
                    }
                }
                {
                    let mut flight = state.flight_lock.lock().unwrap();
                    *flight = None;
                }
                let vault = state.record_vault.lock().unwrap();
                let results: Vec<OmRecordDTO> = vault.iter()
                    .filter(|r| r.puzzle.id == puzzle_id)
                    .cloned()
                    .collect();
                println!("[RESULT]: API returned {} records.", results.len());
                return Ok(results);
            }
            Err(e) => {
                let preview = &body[..body.len().min(300)];
                api_error = Some(format!("JSON error at line {} col {}: {}.  Body: {}...", e.line(), e.column(), e, preview));
                eprintln!("[ZLBB_ERROR]: {}", api_error.as_ref().unwrap());
            }
        }
    }`;

const newBlock = `    if let Some(body) = saved_body {
        let mut remote_records: Vec<OmRecordDTO> = Vec::new();
        let mut raw_count = 0usize;
        let mut skipped = 0usize;
        if let Ok(values) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
            raw_count = values.len();
            if let Some(first) = values.first() {
                let preview = serde_json::to_string(first).unwrap_or_default();
                println!("[ZLBB_DEBUG]: first record preview: {}...", &preview[..preview.len().min(200)]);
            }
            for (i, v) in values.into_iter().enumerate() {
                match serde_json::from_value::<OmRecordDTO>(v) {
                    Ok(record) => remote_records.push(record),
                    Err(e) => { skipped += 1; if skipped <= 3 { eprintln!("[ZLBB_SKIP]: record[{}] parse failed: {}", i, e); } }
                }
            }
        } else {
            match serde_json::from_str::<Vec<OmRecordDTO>>(&body) {
                Ok(records) => { raw_count = records.len(); remote_records = records; }
                Err(e) => {
                    let preview = &body[..body.len().min(300)];
                    api_error = Some(format!("JSON error: {}. Body: {}...", e, preview));
                    eprintln!("[ZLBB_ERROR]: {}", api_error.as_ref().unwrap());
                }
            }
        }
        println!("[ZLBB_PARSER]: {} raw, {} parsed, {} skipped", raw_count, remote_records.len(), skipped);
        if !remote_records.is_empty() {
            {
                let memory_state = app_clone.state::<MemoryState>();
                let puzzle_dto: Option<OmPuzzleDTO> = {
                    let list = memory_state.puzzle_list.lock().unwrap();
                    list.iter().find(|s| s.id == puzzle_id).map(|s| OmPuzzleDTO {
                        id: s.id.clone(), display_name: s.display_name.clone(),
                        r#type: String::new(), group: OmGroupDTO { id: String::new(), display_name: String::new() },
                        alt_ids: Vec::new(),
                    })
                };
                let mut vault = memory_state.record_vault.lock().unwrap();
                for mut remote in remote_records {
                    if remote.puzzle.is_none() { remote.puzzle = puzzle_dto.clone(); }
                    let exists = vault.iter().any(|local|
                        local.puzzle.as_ref().zip(remote.puzzle.as_ref()).map_or(false, |(lp, rp)| lp.id == rp.id)
                            && local.full_formatted_score == remote.full_formatted_score
                    );
                    if !exists { vault.push(remote); }
                }
            }
            { let mut flight = state.flight_lock.lock().unwrap(); *flight = None; }
            let vault = state.record_vault.lock().unwrap();
            let results: Vec<OmRecordDTO> = vault.iter()
                .filter(|r| r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id))
                .cloned().collect();
            println!("[RESULT]: returning {} records for puzzle '{}'.", results.len(), puzzle_id);
            return Ok(results);
        } else if let Some(err) = api_error { return Err(err); }
    }`;

if (c.includes('if let Some(body) = saved_body') && !c.includes('[ZLBB_DEBUG]')) {
  const si = c.indexOf('if let Some(body) = saved_body');
  const ti = c.indexOf('    let final_vault = state.record_vault.lock().unwrap();', si);
  if (si >= 0 && ti >= 0) {
    c = c.substring(0, si) + newBlock + '\n\n' + c.substring(ti);
    console.log('elastic parse: OK');
  } else { console.log('elastic parse: anchors not found'); }
} else { console.log('elastic parse: already done'); }

// === 6. Fix double puzzle.id access ===
c = c.replace(
  'local.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id) == remote.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)',
  'local.puzzle.as_ref().zip(remote.puzzle.as_ref()).map_or(false, |(lp, rp)| lp.id == rp.id)'
);

// === 7. Add utc_now_iso and helpers, update timestamps ===
if (!c.includes('fn utc_now_iso')) {
  c = c.replace(
    'fn utc_now() -> String {',
    'fn utc_now_iso() -> String {\n    chrono::Utc::now().to_rfc3339()\n}\n\nfn utc_now() -> String {'
  );
}

// Update save_cache_meta
c = c.replace(
  'fn save_cache_meta(app: &tauri::AppHandle) {\n    if let Ok(dir) = app.path().app_cache_dir() {\n        let now = utc_now();\n        let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\\"updated\\":\\"{}\\"}}", now));\n    }\n}',
  'fn save_cache_meta(app: &tauri::AppHandle) {\n    if let Ok(dir) = app.path().app_cache_dir() {\n        let now = utc_now_iso();\n        let _ = std::fs::write(dir.join("cache_meta.json"), format!("{{\\"updated\\":\\"{}\\"}}", now));\n    }\n}'
);

// Update get_cache_info to parse ISO 8601
c = c.replace(
  'fn get_cache_info(app: tauri::AppHandle) -> String {\n    let dir = app.path().app_cache_dir().unwrap_or_else(|_| PathBuf::from("."));\n    let meta_path = dir.join("cache_meta.json");\n    let local = std::fs::read_to_string(&meta_path).ok()\n        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())\n        .and_then(|v| v.get("updated").cloned())\n        .and_then(|v| v.as_str().map(|s| format!("Local: {}", s)))\n        .unwrap_or_else(|| {\n            let now = utc_now();\n            let _ = std::fs::write(&meta_path, format!("{{\\"updated\\":\\"{}\\"}}", now));\n            format!("Local: {} (new)", now)\n        });\n    local\n}',
  'fn get_cache_info(app: tauri::AppHandle) -> String {\n    let dir = app.path().app_cache_dir().unwrap_or_else(|_| PathBuf::from("."));\n    let meta_path = dir.join("cache_meta.json");\n    let local = std::fs::read_to_string(&meta_path).ok()\n        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())\n        .and_then(|v| v.get("updated").cloned())\n        .and_then(|v| v.as_str().map(|raw| {\n            if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(raw) {\n                format!("Local: {}", dt.format("%Y-%m-%d %H:%M:%S UTC"))\n            } else {\n                format!("Local: {}", raw)\n            }\n        }))\n        .unwrap_or_else(|| {\n            let now = utc_now_iso();\n            let _ = std::fs::write(&meta_path, format!("{{\\"updated\\":\\"{}\\"}}", now));\n            format!("Local: {} (new)", utc_now())\n        });\n    local\n}'
);

// === 8. Insert ALL new types + sync_incremental + judge_draft before the first cache command ===
// Find the exact insertion point: after save_cache_meta, before "// ============= 5"
const marker8 = '\n\n// ================= 5. ZLBB 跨游戏模糊检索提示命令 =================';
if (!c.includes('fn sync_incremental')) {
  const insert = `

fn read_last_sync_time(app: &tauri::AppHandle) -> Option<String> {
    let dir = app.path().app_cache_dir().ok()?;
    let meta_path = dir.join("cache_meta.json");
    let content = std::fs::read_to_string(&meta_path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("updated")?.as_str().map(|s| s.to_string())
}

// ================= 增量同步结构 =================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmRecordChange {
    pub r#type: String,
    pub record: OmRecordDTO,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    pub new_count: usize, pub removed_count: usize,
    pub synced_until: String, pub errors: Vec<String>,
}

// ================= Pareto 判定面板类型 =================

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InputMode { GCA, GCAI }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParetoJudgeStatus { Ok, Unknown, AlreadyPresented, NothingBeaten }

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OmDraftInput {
    pub cost: Option<i32>, pub cycles: Option<i32>,
    pub area: Option<i32>, pub instructions: Option<i32>,
    pub overlap: bool, pub trackless: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeatenMetricDiff {
    pub actual_value: i32, pub absolute_diff: i32,
    pub percentage_diff: f64, pub formatted_string: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParetoBeatenReport {
    pub better_record: OmRecordDTO,
    pub cost_diff: Option<BeatenMetricDiff>, pub cycles_diff: Option<BeatenMetricDiff>,
    pub area_diff: Option<BeatenMetricDiff>, pub instructions_diff: Option<BeatenMetricDiff>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JudgeResult {
    pub status: ParetoJudgeStatus, pub total_compared: usize,
    pub report: Option<ParetoBeatenReport>,
}

// ================= 增量同步引擎 =================

#[tauri::command]
async fn sync_incremental(
    since: Option<String>, controller: Option<String>,
    state: tauri::State<'_, MemoryState>, app: tauri::AppHandle
) -> Result<SyncResult, String> {
    let ctrl = controller.unwrap_or_else(|| "om".to_string());
    let mut errors: Vec<String> = Vec::new();
    let mut added = 0usize; let mut removed = 0usize;
    let since = since.or_else(|| read_last_sync_time(&app))
        .unwrap_or_else(|| (chrono::Utc::now() - chrono::Duration::hours(24)).to_rfc3339());
    println!("[SYNC]: controller={}, since={}", ctrl, since);
    let client = build_client();
    let base = "https://zlbb.faendir.com";
    let enc = urlencoding::encode(&since);
    {
        let url = format!("{}/{}/records/new/{}", base, ctrl, enc);
        match client.get(&url).send().await {
            Ok(res) if res.status().is_success() => {
                let mut raw = 0usize; let mut parsed: Vec<OmRecordDTO> = Vec::new();
                if let Ok(body) = res.text().await {
                    if let Ok(vals) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
                        raw = vals.len();
                        for v in vals { if let Ok(r) = serde_json::from_value::<OmRecordDTO>(v) { parsed.push(r); } }
                    }
                }
                if !parsed.is_empty() {
                    let mut vault = state.record_vault.lock().unwrap();
                    for r in &parsed {
                        let exists = vault.iter().any(|v| v.id.as_deref() == r.id.as_deref()
                            && v.full_formatted_score.as_deref() == r.full_formatted_score.as_deref());
                        if !exists { vault.push(r.clone()); added += 1; }
                    }
                }
                println!("[SYNC_NEW]: {} raw, {} ok, {} added", raw, parsed.len(), added);
            }
            Ok(res) => errors.push(format!("new HTTP {}", res.status())),
            Err(e) => errors.push(format!("new fetch: {}", e)),
        }
    }
    {
        let url = format!("{}/{}/records/changes/{}", base, ctrl, enc);
        match client.get(&url).send().await {
            Ok(res) if res.status().is_success() => {
                let mut raw = 0usize; let mut parsed: Vec<OmRecordChange> = Vec::new();
                if let Ok(body) = res.text().await {
                    if let Ok(vals) = serde_json::from_str::<Vec<serde_json::Value>>(&body) {
                        raw = vals.len();
                        for v in vals { if let Ok(ch) = serde_json::from_value::<OmRecordChange>(v) { parsed.push(ch); } }
                    }
                }
                let mut vault = state.record_vault.lock().unwrap();
                for ch in &parsed {
                    match ch.r#type.as_str() {
                        "ADD" => {
                            let exists = vault.iter().any(|v| v.id.as_deref() == ch.record.id.as_deref()
                                && v.full_formatted_score.as_deref() == ch.record.full_formatted_score.as_deref());
                            if !exists { vault.push(ch.record.clone()); added += 1; }
                        }
                        "REMOVE" => {
                            if let Some(ref rid) = ch.record.id {
                                let before = vault.len();
                                vault.retain(|v| v.id.as_deref() != Some(rid.as_str()));
                                removed += before - vault.len();
                            }
                        }
                        _ => {}
                    }
                }
                println!("[SYNC_CHANGES]: {} raw, {} ok", raw, parsed.len());
            }
            Ok(res) => errors.push(format!("changes HTTP {}", res.status())),
            Err(e) => errors.push(format!("changes fetch: {}", e)),
        }
    }
    let synced_until = utc_now_iso();
    save_cache_meta(&app);
    println!("[SYNC_DONE]: +{} -{} until={}", added, removed, synced_until);
    Ok(SyncResult { new_count: added, removed_count: removed, synced_until, errors })
}

// ================= Pareto 实时判定引擎 =================

fn is_dominated_by(
    a_cost: Option<i32>, a_cycles: Option<i32>, a_area: Option<i32>, a_instructions: Option<i32>,
    b_cost: i32, b_cycles: i32, b_area: i32, b_instructions: i32, dims: &[&str],
) -> bool {
    let mut any_worse = false;
    for d in dims { match *d {
        "cost" => { let a = a_cost.unwrap_or(i32::MAX); if a < b_cost { return false; } if a > b_cost { any_worse = true; } }
        "cycles" => { let a = a_cycles.unwrap_or(i32::MAX); if a < b_cycles { return false; } if a > b_cycles { any_worse = true; } }
        "area" => { let a = a_area.unwrap_or(i32::MAX); if a < b_area { return false; } if a > b_area { any_worse = true; } }
        "instructions" => { let a = a_instructions.unwrap_or(i32::MAX); if a < b_instructions { return false; } if a > b_instructions { any_worse = true; } }
        _ => {}
    }}
    any_worse
}

fn is_exact_match(
    a_cost: Option<i32>, a_cycles: Option<i32>, a_area: Option<i32>, a_instructions: Option<i32>,
    b_cost: i32, b_cycles: i32, b_area: i32, b_instructions: i32, dims: &[&str],
) -> bool {
    for d in dims { match *d {
        "cost" => { if a_cost != Some(b_cost) { return false; } }
        "cycles" => { if a_cycles != Some(b_cycles) { return false; } }
        "area" => { if a_area != Some(b_area) { return false; } }
        "instructions" => { if a_instructions != Some(b_instructions) { return false; } }
        _ => {}
    }}
    true
}

fn make_diff(user_val: i32, best_val: i32) -> BeatenMetricDiff {
    let abs_diff = user_val - best_val;
    let pct = if best_val > 0 { (user_val as f64) / (best_val as f64) * 100.0 } else { f64::INFINITY };
    BeatenMetricDiff { actual_value: best_val, absolute_diff: abs_diff, percentage_diff: pct,
        formatted_string: format!("{} ({:+} / {:.2}%)", best_val, -abs_diff, pct) }
}

#[tauri::command]
async fn judge_draft(
    draft: OmDraftInput, mode: InputMode, puzzle_id: String,
    state: tauri::State<'_, MemoryState>,
) -> Result<JudgeResult, String> {
    let dims: Vec<&str> = match mode {
        InputMode::GCA => vec!["cost", "cycles", "area"],
        InputMode::GCAI => vec!["cost", "cycles", "area", "instructions"],
    };
    if (dims.contains(&"cost") && draft.cost.is_none())
        || (dims.contains(&"cycles") && draft.cycles.is_none())
        || (dims.contains(&"area") && draft.area.is_none())
        || (dims.contains(&"instructions") && draft.instructions.is_none())
    { return Ok(JudgeResult { status: ParetoJudgeStatus::Unknown, total_compared: 0, report: None }); }
    let vault = state.record_vault.lock().unwrap();
    let candidates: Vec<OmRecordDTO> = vault.iter().filter(|r| {
        r.puzzle.as_ref().map_or(false, |p| p.id == puzzle_id)
            && r.score.is_some()
            && r.score.as_ref().unwrap().overlap == draft.overlap
            && r.score.as_ref().unwrap().trackless == draft.trackless
    }).cloned().collect();
    let total = candidates.len();
    if total == 0 { return Ok(JudgeResult { status: ParetoJudgeStatus::Ok, total_compared: 0, report: None }); }
    for r in &candidates {
        let s = r.score.as_ref().unwrap();
        if is_exact_match(draft.cost, draft.cycles, draft.area, draft.instructions,
            s.cost, s.cycles, s.area, s.instructions, &dims)
        { return Ok(JudgeResult { status: ParetoJudgeStatus::AlreadyPresented, total_compared: total, report: None }); }
    }
    let dominators: Vec<&OmRecordDTO> = candidates.iter().filter(|r| {
        let s = r.score.as_ref().unwrap();
        is_dominated_by(draft.cost, draft.cycles, draft.area, draft.instructions,
            s.cost, s.cycles, s.area, s.instructions, &dims)
    }).collect();
    if dominators.is_empty() { return Ok(JudgeResult { status: ParetoJudgeStatus::Ok, total_compared: total, report: None }); }
    let best = dominators.iter().fold(None, |acc: Option<(i32,i32,i32,i32)>, r| {
        let s = r.score.as_ref().unwrap();
        match acc { None => Some((s.cost, s.cycles, s.area, s.instructions)),
            Some((c,cy,a,i)) => Some((c.min(s.cost), cy.min(s.cycles), a.min(s.area), i.min(s.instructions))) }
    });
    let rep = dominators.iter().max_by(|a,b| {
        let sa=a.score.as_ref().unwrap(); let sb=b.score.as_ref().unwrap();
        (sb.cost+sb.cycles+sb.area+sb.instructions).cmp(&(sa.cost+sa.cycles+sa.area+sa.instructions))
    });
    let report = if let (Some((b_c,b_cy,b_a,b_i)), Some(rep_r)) = (best, rep) {
        println!("[JUDGE]: NothingBeaten by {} recs best=({},{},{},{})", dominators.len(), b_c,b_cy,b_a,b_i);
        Some(ParetoBeatenReport { better_record: rep_r.clone(),
            cost_diff: if dims.contains(&"cost") { Some(make_diff(draft.cost.unwrap_or(0), b_c)) } else { None },
            cycles_diff: if dims.contains(&"cycles") { Some(make_diff(draft.cycles.unwrap_or(0), b_cy)) } else { None },
            area_diff: if dims.contains(&"area") { Some(make_diff(draft.area.unwrap_or(0), b_a)) } else { None },
            instructions_diff: if dims.contains(&"instructions") { Some(make_diff(draft.instructions.unwrap_or(0), b_i)) } else { None },
        })
    } else { None };
    Ok(JudgeResult { status: ParetoJudgeStatus::NothingBeaten, total_compared: total, report })
}`;
  c = c.replace(marker8, insert + marker8);
  console.log('sync+judge: inserted');
} else { console.log('sync+judge: already present'); }

// === 9. Update handler to register all commands ===
c = c.replace(
  '.invoke_handler(tauri::generate_handler![search_om_records, get_live_puzzle_suggestions, check_boot_ready, get_cache_path, get_cache_info])',
  '.invoke_handler(tauri::generate_handler![search_om_records, sync_incremental, judge_draft, get_live_puzzle_suggestions, check_boot_ready, get_cache_path, get_cache_info])'
);

fs.writeFileSync('src/lib.rs', c);
console.log('DONE, length=' + c.length);
