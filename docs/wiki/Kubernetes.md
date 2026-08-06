# Kubernetes

The Kubernetes tab (`Alt+5`) shows Pods, Nodes and Deployments through
[kube-rs](https://github.com/kube-rs/kube). It is **read-only by
construction** — `LIST` on those three resources plus `GET` on
`metrics.k8s.io/v1beta1`, and nothing else. No `CREATE`, `UPDATE`, `DELETE` or
`PATCH` is ever issued.

| Key | |
|---|---|
| `P` / `N` / `D` | Pods / Nodes / Deployments |
| `]` / `[` | Cycle sub-views |
| `A` | Toggle one namespace ↔ **A**ll namespaces (local mode) |
| `s` / `S` | Sort / reverse |
| `/` | Filter |
| `Enter` | Inspect — full pod name, node, containers |

---

## Credentials

With no flags, in this order:

1. `$KUBECONFIG`
2. `~/.kube/config`
3. the in-cluster ServiceAccount (`/var/run/secrets/kubernetes.io/serviceaccount`)

```sh
muxtop --kube-context staging        # a specific context
muxtop --kube-namespace production   # scope Pods and Deployments
muxtop --no-kube                     # disable cluster collection entirely
```

**The kubeconfig never crosses the wire.** In `--remote` mode the server is the
only side that opens it; clients receive digested snapshots. The server's
`--kube-context` / `--kube-namespace` decide what exists, and `A` does nothing
remotely.

Cluster data is polled at **0.2 Hz** — every five seconds. API server round
trips are the slowest thing muxtop does and the least likely to change between
frames.

---

## Permissions: the part that surprises people

**By default muxtop lists all three resources cluster-wide**, which needs
cluster-scoped `list`. On a shared cluster you usually do not have that, and the
tab looks broken when it is really just being denied.

### Scoping to one namespace

```sh
muxtop --kube-namespace my-namespace
```

Pods and Deployments are then listed from that namespace only, which works with
a plain `Role` — no cluster-wide grant. Press `A` to switch between the scoped
and cluster-wide views at runtime.

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: muxtop-readonly
  namespace: my-namespace
rules:
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["list"]
  - apiGroups: ["apps"]
    resources: ["deployments"]
    verbs: ["list"]
  - apiGroups: ["metrics.k8s.io"]
    resources: ["pods"]
    verbs: ["list"]
```

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: muxtop-readonly
  namespace: my-namespace
subjects:
  - kind: ServiceAccount
    name: muxtop
    namespace: my-namespace
roleRef:
  kind: Role
  name: muxtop-readonly
  apiGroup: rbac.authorization.k8s.io
```

### Nodes are different, and always will be

**`Node` is a cluster-scoped resource in Kubernetes** — there is no namespaced
variant. So the Nodes sub-view needs cluster-wide `list` no matter what
`--kube-namespace` says, and renders empty without it. Pods and Deployments are
unaffected.

The same split applies to metrics: pod CPU/MEM follows the namespace scope, node
CPU/MEM does not.

If you want Nodes, you need a `ClusterRole`:

```yaml
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: muxtop-readonly
rules:
  - apiGroups: [""]
    resources: ["pods", "nodes"]
    verbs: ["list"]
  - apiGroups: ["apps"]
    resources: ["deployments"]
    verbs: ["list"]
  - apiGroups: ["metrics.k8s.io"]
    resources: ["pods", "nodes"]
    verbs: ["list"]
```

Bind it with a `ClusterRoleBinding` to the ServiceAccount or user muxtop runs
as. Note there is no `get`, no `watch` and no write verb in that list — `list`
is all muxtop uses.

---

## metrics-server

CPU and MEM columns come from `metrics.k8s.io`, which is **not** part of a
stock Kubernetes install. Without
[metrics-server](https://github.com/kubernetes-sigs/metrics-server), those
columns render `—` and everything else works normally.

That is deliberate: `—` means *"this cluster cannot tell me"*, not *"this pod
uses no CPU"*. Reporting `0` there would make the tab lie about a busy pod.

```sh
kubectl top nodes    # if this works, muxtop's CPU/MEM columns will populate
```

On managed clusters metrics-server is often preinstalled (GKE, AKS, EKS with the
add-on). On `kind` and `k3d` it usually is not.

---

## Running muxtop inside the cluster

The in-cluster ServiceAccount is picked up automatically, so a pod needs no
kubeconfig:

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: muxtop
  namespace: monitoring
---
# Bind the ClusterRole above to this ServiceAccount, then:
apiVersion: v1
kind: Pod
metadata:
  name: muxtop
  namespace: monitoring
spec:
  serviceAccountName: muxtop
  containers:
    - name: muxtop
      image: debian:stable-slim   # install muxtop, or build your own image
      command: ["muxtop"]
      tty: true
      stdin: true
      resources:
        requests: { cpu: 50m, memory: 32Mi }
        limits:   { cpu: 500m, memory: 64Mi }
      securityContext:
        runAsNonRoot: true
        runAsUser: 65534
        allowPrivilegeEscalation: false
        readOnlyRootFilesystem: true
        capabilities: { drop: ["ALL"] }
```

Then `kubectl exec -it -n monitoring muxtop -- muxtop`. Note that a container's
Processes tab shows the **container's** PID namespace, not the node's — which is
usually one process. To watch a node, run muxtop on the node.

---

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| *"No cluster"* | No kubeconfig found and no in-cluster ServiceAccount. Check `$KUBECONFIG` and `kubectl config current-context` |
| Pods list, Nodes empty | Expected without cluster-wide `list` on nodes — see above. Not a bug |
| Everything empty, `kubectl` works | muxtop reads the **active** context. `kubectl config use-context <name>`, or pass `--kube-context` |
| CPU/MEM all `—` | metrics-server is absent. Check with `kubectl top pods` |
| Node CPU/MEM `—` while pod values work | Node metrics are cluster-scoped; your grant is namespaced |
| Empty over `--remote` | The **server**'s kubeconfig or `--no-kube`, not the client's |
| Forbidden errors in the log | RBAC. The log names the resource that was denied |

Detail lands in `~/.local/share/muxtop/muxtop.log`; `MUXTOP_LOG=debug muxtop`
logs the credential discovery sequence and the API responses.

---

## Not implemented

Write actions — delete pod, scale deployment, rollout restart — are
**explicitly out of scope**. The read-only guarantee is a feature: muxtop can be
handed a credential without worrying about what it might do with it, and
"read-only by construction" is a claim you can verify by grepping for the verbs
it issues.

Also absent: events, logs, CRDs, StatefulSets and DaemonSets. Pods, Nodes and
Deployments cover the "is this cluster healthy" question the tab exists to
answer.
