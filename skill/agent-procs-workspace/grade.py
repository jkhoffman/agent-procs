#!/usr/bin/env python3
"""Grade agent-procs skill eval outputs against assertions."""

import json
import yaml
import re
import os

WORKSPACE = os.path.dirname(os.path.abspath(__file__))
ITER = os.path.join(WORKSPACE, "iteration-1")


def load_yaml(path):
    try:
        with open(path) as f:
            return yaml.safe_load(f)
    except Exception:
        return None


def load_text(path):
    try:
        with open(path) as f:
            return f.read()
    except Exception:
        return ""


def grade_eval1(variant):
    """Full-stack config."""
    base = os.path.join(ITER, "full-stack-config", variant, "outputs")
    cfg = load_yaml(os.path.join(base, "agent-procs.yaml"))
    cmds = load_text(os.path.join(base, "commands.sh"))

    results = []
    procs = cfg.get("processes", {}) if cfg else {}
    proc_names = set(procs.keys())

    # config_has_three_processes
    has_three = len(procs) >= 3
    has_db = any(k in proc_names for k in ["db", "database", "postgres"])
    has_api = any(k in proc_names for k in ["api", "backend"])
    has_web = any(k in proc_names for k in ["web", "frontend"])
    results.append({
        "text": "config_has_three_processes",
        "passed": has_three and has_db and has_api and has_web,
        "evidence": f"Found processes: {list(proc_names)}"
    })

    # config_has_dependency_chain
    api_key = next((k for k in proc_names if k in ["api", "backend"]), None)
    web_key = next((k for k in proc_names if k in ["web", "frontend"]), None)
    db_key = next((k for k in proc_names if k in ["db", "database", "postgres"]), None)
    api_deps = procs.get(api_key, {}).get("depends_on", []) if api_key else []
    web_deps = procs.get(web_key, {}).get("depends_on", []) if web_key else []
    results.append({
        "text": "config_has_dependency_chain",
        "passed": (db_key in api_deps if db_key else False) and (api_key in web_deps if api_key else False),
        "evidence": f"api depends_on={api_deps}, web depends_on={web_deps}"
    })

    # config_has_env_vars
    api_env = procs.get(api_key, {}).get("env", {}) if api_key else {}
    web_env = procs.get(web_key, {}).get("env", {}) if web_key else {}
    has_db_url = "DATABASE_URL" in api_env
    has_vite = "VITE_API_URL" in web_env
    results.append({
        "text": "config_has_env_vars",
        "passed": has_db_url and has_vite,
        "evidence": f"api env keys={list(api_env.keys())}, web env keys={list(web_env.keys())}"
    })

    # config_has_ready_patterns
    all_ready = all(procs[k].get("ready") for k in procs)
    results.append({
        "text": "config_has_ready_patterns",
        "passed": all_ready,
        "evidence": f"ready values: {[procs[k].get('ready', '<MISSING>') for k in procs]}"
    })

    # config_has_cwd
    api_cwd = procs.get(api_key, {}).get("cwd", "") if api_key else ""
    web_cwd = procs.get(web_key, {}).get("cwd", "") if web_key else ""
    results.append({
        "text": "config_has_cwd",
        "passed": "api" in api_cwd and "web" in web_cwd,
        "evidence": f"api cwd={api_cwd}, web cwd={web_cwd}"
    })

    # commands_use_up
    results.append({
        "text": "commands_use_up",
        "passed": "agent-procs up" in cmds,
        "evidence": f"Found 'agent-procs up' in commands: {'agent-procs up' in cmds}"
    })

    # commands_verify_status
    results.append({
        "text": "commands_verify_status",
        "passed": "agent-procs status" in cmds,
        "evidence": f"Found 'agent-procs status' in commands: {'agent-procs status' in cmds}"
    })

    return results


def grade_eval2(variant):
    """Background server testing."""
    base = os.path.join(ITER, "background-server-testing", variant, "outputs")
    cmds = load_text(os.path.join(base, "commands.sh"))

    results = []

    # uses_agent_procs_run — check actual command lines (not comments)
    cmd_lines = [l.strip() for l in cmds.splitlines() if l.strip() and not l.strip().startswith("#")]
    uses_run = any("agent-procs run" in l for l in cmd_lines)
    uses_nohup = any("nohup" in l for l in cmd_lines)
    uses_ampersand = any(re.search(r'&\s*$', l) for l in cmd_lines)
    results.append({
        "text": "uses_agent_procs_run",
        "passed": uses_run and not uses_nohup and not uses_ampersand,
        "evidence": f"agent-procs run: {uses_run}, nohup: {uses_nohup}, trailing &: {uses_ampersand}"
    })

    # gives_process_a_name
    has_name = "--name" in cmds
    results.append({
        "text": "gives_process_a_name",
        "passed": has_name,
        "evidence": f"--name flag present: {has_name}"
    })

    # uses_wait_with_pattern
    has_wait = "agent-procs wait" in cmds
    has_until = "--until" in cmds
    results.append({
        "text": "uses_wait_with_pattern",
        "passed": has_wait and has_until,
        "evidence": f"agent-procs wait: {has_wait}, --until: {has_until}"
    })

    # has_timeout
    timeout_match = re.search(r'--timeout\s+(\d+)', cmds)
    timeout_val = int(timeout_match.group(1)) if timeout_match else 0
    results.append({
        "text": "has_timeout",
        "passed": timeout_val >= 10,
        "evidence": f"timeout value: {timeout_val}"
    })

    # runs_tests_after_wait
    wait_pos = cmds.find("agent-procs wait")
    test_patterns = ["test:integration", "test test/integration", "rspec", "pytest"]
    test_pos = -1
    for pat in test_patterns:
        pos = cmds.find(pat)
        if pos > 0:
            test_pos = pos
            break
    results.append({
        "text": "runs_tests_after_wait",
        "passed": wait_pos >= 0 and test_pos > wait_pos,
        "evidence": f"wait at pos {wait_pos}, test at pos {test_pos}"
    })

    # cleans_up
    has_stop = "agent-procs stop" in cmds or "agent-procs down" in cmds
    results.append({
        "text": "cleans_up",
        "passed": has_stop,
        "evidence": f"stop/down command present: {has_stop}"
    })

    return results


def grade_eval3(variant):
    """Microservices topology."""
    base = os.path.join(ITER, "microservices-topology", variant, "outputs")
    cfg = load_yaml(os.path.join(base, "agent-procs.yaml"))
    cmds = load_text(os.path.join(base, "commands.sh"))

    results = []
    procs = cfg.get("processes", {}) if cfg else {}
    proc_names = set(procs.keys())

    # config_has_three_services
    has_three = len(procs) >= 3
    has_gateway = any("gateway" in k for k in proc_names)
    has_users = any("user" in k for k in proc_names)
    has_orders = any("order" in k for k in proc_names)
    results.append({
        "text": "config_has_three_services",
        "passed": has_three and has_gateway and has_users and has_orders,
        "evidence": f"Found processes: {list(proc_names)}"
    })

    # gateway_depends_on_both
    gw_key = next((k for k in proc_names if "gateway" in k), None)
    users_key = next((k for k in proc_names if "user" in k), None)
    orders_key = next((k for k in proc_names if "order" in k), None)
    gw_deps = procs.get(gw_key, {}).get("depends_on", []) if gw_key else []
    deps_both = (users_key in gw_deps and orders_key in gw_deps) if (users_key and orders_key) else False
    results.append({
        "text": "gateway_depends_on_both",
        "passed": deps_both,
        "evidence": f"gateway depends_on={gw_deps}"
    })

    # leaf_services_independent
    users_deps = procs.get(users_key, {}).get("depends_on", []) if users_key else []
    orders_deps = procs.get(orders_key, {}).get("depends_on", []) if orders_key else []
    results.append({
        "text": "leaf_services_independent",
        "passed": len(users_deps) == 0 and len(orders_deps) == 0,
        "evidence": f"users depends_on={users_deps}, orders depends_on={orders_deps}"
    })

    # config_has_ready_patterns
    all_ready = all(procs[k].get("ready") for k in procs)
    results.append({
        "text": "config_has_ready_patterns",
        "passed": all_ready,
        "evidence": f"ready values: {[procs[k].get('ready', '<MISSING>') for k in procs]}"
    })

    # config_has_cwd
    all_cwd = all(
        "services" in (procs[k].get("cwd", "") or "")
        for k in procs
    )
    results.append({
        "text": "config_has_cwd",
        "passed": all_cwd,
        "evidence": f"cwds: {[procs[k].get('cwd', '<MISSING>') for k in procs]}"
    })

    # commands_use_up
    results.append({
        "text": "commands_use_up",
        "passed": "agent-procs up" in cmds,
        "evidence": f"Found 'agent-procs up' in commands: {'agent-procs up' in cmds}"
    })

    return results


def save_grading(eval_name, variant, results):
    path = os.path.join(ITER, eval_name, variant, "grading.json")
    passed = sum(1 for r in results if r["passed"])
    total = len(results)
    data = {
        "eval_name": eval_name,
        "variant": variant,
        "pass_rate": passed / total if total > 0 else 0,
        "passed": passed,
        "total": total,
        "expectations": results
    }
    with open(path, "w") as f:
        json.dump(data, f, indent=2)
    print(f"{eval_name}/{variant}: {passed}/{total} ({data['pass_rate']:.0%})")


if __name__ == "__main__":
    for variant in ["with_skill", "without_skill"]:
        save_grading("full-stack-config", variant, grade_eval1(variant))
        save_grading("background-server-testing", variant, grade_eval2(variant))
        save_grading("microservices-topology", variant, grade_eval3(variant))
