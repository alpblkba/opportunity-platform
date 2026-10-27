//! Local, evidence-preserving CV extraction. No private text leaves this process.
use opportunity_domain::normalize::contains_phrase;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CandidateProfile {
    pub name: Option<String>,
    pub email: Option<String>,
    pub phone: Option<String>,
    pub education: Vec<String>,
    pub employment: Vec<String>,
    pub projects: Vec<String>,
    pub skills: Vec<String>,
    pub experience_years: Option<i32>,
    pub degree: Option<String>,
}

const SKILLS: &[&str] = &[
    "C",
    "C++",
    "Rust",
    "Python",
    "Java",
    "Linux",
    "RTOS",
    "firmware",
    "embedded",
    "device driver",
    "kernel",
    "operating systems",
    "memory",
    "DRAM",
    "SRAM",
    "CXL",
    "PCIe",
    "DDR",
    "cache",
    "virtual memory",
    "NUMA",
    "RISC-V",
    "ARM",
    "x86",
    "RTL",
    "Verilog",
    "SystemVerilog",
    "VHDL",
    "FPGA",
    "ASIC",
    "UVM",
    "verification",
    "microarchitecture",
    "CPU",
    "GPU",
    "CUDA",
    "PyTorch",
    "TensorFlow",
    "machine learning",
    "compiler",
    "LLVM",
    "AI",
    "accelerator",
    "distributed systems",
    "performance",
    "DSP",
    "MATLAB",
    "JTAG",
    "I2C",
    "SPI",
    "UART",
    "bootloader",
    "security",
];

pub fn mentions(text: &str, term: &str) -> bool {
    // Preserve C++: the general normalizer intentionally removes punctuation.
    if term == "C++" {
        return text.to_lowercase().contains("c++");
    }
    if term == "C" {
        return text
            .split(|c: char| !c.is_alphanumeric() && c != '+')
            .any(|w| w == "C" || w == "c");
    }
    contains_phrase(
        &opportunity_domain::normalize_location(Some(text)),
        &opportunity_domain::normalize_location(Some(term)),
    )
}

pub fn extract(text: &str) -> Result<CandidateProfile, String> {
    if text.trim().chars().count() < 40 {
        return Err("No readable CV text found. Upload a text PDF or a .txt CV.".into());
    }
    let mut out = CandidateProfile::default();
    let mut section = "";
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let heading = line.trim_end_matches(':').to_lowercase();
        match heading.as_str() {
            "education"
            | "academic background"
            | "education & qualifications"
            | "eğitim"
            | "ausbildung" => {
                section = "education";
                continue;
            }
            "experience"
            | "work experience"
            | "professional experience"
            | "employment"
            | "iş deneyimi"
            | "berufserfahrung" => {
                section = "employment";
                continue;
            }
            "projects" | "selected projects" | "research experience" | "research" | "projeler" => {
                section = "projects";
                continue;
            }
            "skills" | "technical skills" | "technologies" | "publications" | "languages"
            | "certifications" | "interests" | "summary" | "profile" => {
                section = "";
                continue;
            }
            _ => (),
        }
        let value = line.chars().take(600).collect::<String>();
        match section {
            "education" => out.education.push(value),
            "employment" => out.employment.push(value),
            "projects" => out.projects.push(value),
            _ => (),
        }
        for word in line.split_whitespace() {
            let word = word.trim_matches(|c: char| !c.is_alphanumeric() && !"@._+-".contains(c));
            if word.matches('@').count() == 1
                && word.split('@').nth(1).is_some_and(|d| d.contains('.'))
                && out.email.is_none()
            {
                out.email = Some(word.to_owned());
            }
        }
        let lower = line.to_lowercase();
        for prefix in ["phone:", "tel:", "mobile:"] {
            if lower.starts_with(prefix) {
                let phone = line[prefix.len()..].trim();
                if phone.chars().filter(char::is_ascii_digit).count() >= 7
                    && phone
                        .chars()
                        .all(|c| c.is_ascii_digit() || "+- ().".contains(c))
                {
                    out.phone = Some(phone.to_owned());
                }
            }
        }
        // Only explicit claims; dates of education or projects never become employment years.
        let words: Vec<&str> = lower.split_whitespace().collect();
        for window in words.windows(4) {
            if window[1] == "years" && window[2] == "of" && window[3].starts_with("experience") {
                out.experience_years = window[0]
                    .trim_end_matches('+')
                    .parse::<i32>()
                    .ok()
                    .filter(|n| (0..=60).contains(n));
            }
        }
    }
    if let Some(first) = text.lines().map(str::trim).find(|l| !l.is_empty()) {
        let name = first.strip_prefix("Name:").unwrap_or(first).trim();
        if (2..=5).contains(&name.split_whitespace().count())
            && name
                .chars()
                .all(|c| c.is_alphabetic() || " .'-".contains(c))
            && ![
                "resume",
                "curriculum",
                "engineer",
                "developer",
                "profile",
                "skills",
                "education",
                "experience",
            ]
            .iter()
            .any(|w| name.to_lowercase().contains(w))
        {
            out.name = Some(name.to_owned());
        }
    }
    out.skills = SKILLS
        .iter()
        .filter(|s| mentions(text, s))
        .map(|s| (*s).to_owned())
        .collect();
    // An enrolled degree is evidence of study, not an awarded qualification.
    for line in &out.education {
        let lower = line.to_lowercase();
        let future = lower
            .split(|c: char| !c.is_ascii_digit())
            .filter_map(|v| v.parse::<i32>().ok())
            .any(|year| year > chrono::Datelike::year(&chrono::Utc::now()) && year < 2100);
        if future
            || ["expected", "ongoing", "present", "candidate", "pursuing"]
                .iter()
                .any(|s| lower.contains(s))
        {
            continue;
        }
        // A degree name alone proves study, not graduation.
        if !["completed", "graduated", "awarded", "conferred"]
            .iter()
            .any(|s| lower.contains(s))
        {
            continue;
        }
        for (needle, degree) in [
            ("phd", "PhD"),
            ("ph.d", "PhD"),
            ("master", "MSc"),
            ("m.sc", "MSc"),
            ("msc", "MSc"),
            ("bachelor", "BSc"),
            ("b.sc", "BSc"),
            ("bsc", "BSc"),
        ] {
            if lower.contains(needle)
                && (out.degree.is_none()
                    || degree == "PhD"
                    || degree == "MSc" && out.degree.as_deref() == Some("BSc"))
            {
                out.degree = Some(degree.into());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_evidence_without_inventing_eligibility_or_experience() {
        let p = extract("Ada Example\nada@example.test\nEducation\nMSc Computer Science, expected 2027\nExperience\nFirmware intern, 2024\nProjects\nLinux kernel and C++ memory project\nSkills\nPython, DRAM, CXL").unwrap();
        assert_eq!(p.name.as_deref(), Some("Ada Example"));
        assert!(p.skills.contains(&"C++".into()));
        assert!(!p.skills.contains(&"C".into()));
        assert!(p.degree.is_none());
        assert!(p.experience_years.is_none());
        assert_eq!(p.employment.len(), 1);
        assert_eq!(p.projects.len(), 1);
        assert!(extract(" ").is_err());
    }
}
