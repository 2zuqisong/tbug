```markdown
 _____ ____
|_   _| __ ) _   _  __ _ 
  | | |  _ \| | | |/ _` |
  | | | |_) | |_| | (_| |
  |_| |____/ \__,_|\__, |
                   |___/  v0.1.0

```
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Node.js Version](https://img.shields.io/badge/node-%3E%3D%2020.0.0-brightgreen.svg)](https://nodejs.org/)
[![LLM: DeepSeek V4](https://img.shields.io/badge/LLM-DeepSeek%20V4-blue.svg)](https://platform.deepseek.com)
---

## 🚀 Quick Start

### 1. Clone and Build

```bash
git clone <repo-url> && cd tbug
npm install
npm run build

```

### 2. Configure Your API Key

Create a `.env` file in the project root (it is already gitignored):

```env
DEEPSEEK_API_KEY=sk-your-actual-api-key-here

```

### 3. Let Your Bug Hunter Loose

Point tbug at any local project you're working on, from any directory:

```bash
# Have tbug tame a broken Rust project
node /path/to/tbug/dist/main.js cargo check

# Have tbug diagnose a failing Node.js test suite
node /path/to/tbug/dist/main.js npm run test

# Or register it as a global system command via npm link
npm link
tbug make

```

---

## 🎛️ Configuration Reference

| Environment Variable | Required | Default | Description |
| --- | --- | --- | --- |
| `DEEPSEEK_API_KEY` | **Yes** | — | Your DeepSeek platform API key (official or proxy) |
| `DEEPSEEK_API_BASE` | No | `https://api.deepseek.com/v1` | Override the API endpoint for private deployments or third-party proxies |

> 💡 **Note**: Configuration uses three-tier priority resolution: constructor parameter → `.env` file → shell environment. `.env` values take highest precedence.

---

## ⏱️ Version

Current production version: **0.1.0**


## 📄 License

This project is open-sourced under the **[MIT License](https://opensource.org/licenses/MIT)**.