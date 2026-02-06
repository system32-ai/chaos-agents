# Chaos Agents

The goal of chaos agent is to create bunch agents that run in your infrastructure and create chaos eg. stress on CPU, memory and disk, kill random services. 

It works with - 

- Databases
- Kubernetes clusters
- Servers

For eg. in databases, it figures out the schema, in dev mode, creates a load with inserts, updates, selects, changes configurations and checks the impact, revert the configuration when the chaos experiment is done.

For kubernetes clusters, it randomly kills pods, kill nodes, screws up network configuration and reverts it when the experiment is done

For servers, it fills up with data, change random permissions, stop random services, installs random packages, tries to break the server but reverts the configuration back when it is done.

The goal is to make the process agent, a predefine set to skill should be used here. Rest of the stuff is orchestrated by the agent