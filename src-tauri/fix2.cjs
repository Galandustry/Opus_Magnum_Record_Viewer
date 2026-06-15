const fs=require('fs');let c=fs.readFileSync('src/lib.rs','utf8');c=c.replace(/let now = utc_now\(\);\n\s+let _ = std::fs::write\(&meta_path,/g,'XXX');fs.writeFileSync('src/lib.rs',c)
