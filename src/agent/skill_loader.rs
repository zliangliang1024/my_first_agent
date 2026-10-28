use serde_json::{json, Value};
use std::fs;
use std::path::Path;

// 提取出来的技能元数据
pub struct SkillMeta {
    pub name: String,
    pub description: String,
}

// 扫描本地 skills 文件夹，获取所有技能
pub fn scan_skills() -> Vec<SkillMeta> {
    let mut skills = Vec::new();
    let skills_dir = Path::new("skills");
    
    if skills_dir.exists() && skills_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(skills_dir) {
            for entry in entries.flatten() {
                let skill_md = entry.path().join("SKILL.md");
                if skill_md.exists() {
                    if let Ok(content) = fs::read_to_string(&skill_md) {
                        let mut name = String::new();
                        let mut desc = String::new();
                        let mut in_frontmatter = false;

                        // 简单解析 YAML Frontmatter
                        for line in content.lines() {
                            if line.trim() == "---" {
                                if in_frontmatter { break; } // 遇到第二个 --- 结束
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
                            skills.push(SkillMeta { name, description: desc });
                        }
                    }
                }
            }
        }
    }
    skills
}

// ✅ 新增：将技能转化为大模型能看懂的虚拟工具列表
pub fn get_skill_tools(skills: &[SkillMeta]) -> Vec<Value> {
    skills.iter().map(|s| {
        json!({
            "type": "function",
            "function": {
                // 工具名加上 skill_ 前缀，并把中划线转成下划线（因为 OpenAI 规范工具名只能用字母数字下划线）
                "name": format!("skill_{}", s.name.replace("-", "_")),
                "description": format!("[技能] {}", s.description),
                "parameters": { "type": "object", "properties": {} } // 技能不需要传参
            }
        })
    }).collect()
}

// 读取某个具体技能的完整 SOP 内容（留给执行时用）
pub fn read_skill_file(skill_name: &str) -> String {
    let path = format!("skills/{}/SKILL.md", skill_name);
    if let Ok(content) = fs::read_to_string(path) {
        content
    } else {
        format!("未找到名为 {} 的技能文件，请检查 skills 目录。", skill_name)
    }
}