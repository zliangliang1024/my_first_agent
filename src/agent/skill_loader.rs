use std::fs;
use std::path::Path;

pub fn load_agent_context() -> String {
    let mut context = String::new();

    // 1. 读取 AGENTS.md
    if let Ok(content) = fs::read_to_string("AGENTS.md") {
        context.push_str(&format!("\n# 项目规则 (AGENTS.md)\n{}\n", content));
    }

    // 2. 扫描 skills/ 目录
    let skills_dir = Path::new("skills");
    if skills_dir.exists() && skills_dir.is_dir() {
        context.push_str("\n# 可用技能手册\n");
        context.push_str("你可以使用工具 `read_skill` 来读取以下技能的完整内容：\n\n");

        if let Ok(entries) = fs::read_dir(skills_dir) {
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                if skill_md.exists() {
                    if let Ok(content) = fs::read_to_string(&skill_md) {
                        // 简单的提取 YAML Frontmatter 中的 name 和 description
                        let mut name = String::new();
                        let mut desc = String::new();
                        let mut in_frontmatter = false;

                        for line in content.lines() {
                            if line.trim() == "---" {
                                if in_frontmatter { break; } // Frontmatter 结束
                                in_frontmatter = true;
                                continue;
                            }
                            if in_frontmatter {
                                if let Some(n) = line.strip_prefix("name:") {
                                    name = n.trim().to_string();
                                } else if let Some(d) = line.strip_prefix("description:") {
                                    desc = d.trim().to_string();
                                }
                            }
                        }

                        if !name.is_empty() {
                            context.push_str(&format!("- **{}**: {}\n", name, desc));
                        }
                    }
                }
            }
        }
    }

    context
}

// 读取某个具体技能的完整内容
pub fn read_skill_file(skill_name: &str) -> String {
    let path = format!("skills/{}/SKILL.md", skill_name);
    if let Ok(content) = fs::read_to_string(path) {
        content
    } else {
        format!("未找到名为 {} 的技能文件。", skill_name)
    }
}