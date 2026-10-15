# Kubernetes

Run the FastAPI shop's experiments on Kubernetes (GKE, EKS, AKS, or any
other cluster). Each pod of the shop runs `ssp proxy` beside the shop, and one
`ssp controller` pod runs every arm as a copy of the shop's Deployment.

| File | Purpose |
| --- | --- |
| `shop.yaml` | The shop's Deployment with the `statespace-proxy` sidecar, and its Service |
| `statespace-controller.yaml` | The controller, its service account, and what it may do |

## Set up

Create the tokens and the signing key, and store them in a Secret.

```shell
ssp app create shop
ssp keygen                     # the private key goes to CI; the public key below
ssp token create --name shop-proxies --preset runtime
ssp token create --name shop-controller --preset controller
kubectl create secret generic statespace \
  --from-literal=runtime-token=<runtime token> \
  --from-literal=controller-token=<controller token> \
  --from-literal=trust-key=<public key>
kubectl apply -f shop.yaml -f statespace-controller.yaml
```

CI publishes each branch's image as a variant, as `../fastapi/variant.yml`
does. Start an experiment, and the controller starts its arms:

```shell
ssp experiment create price --app shop --split lower-price=0.05
ssp experiment start price
kubectl get deployments -l app.kubernetes.io/managed-by=ssp-controller
```

## What the controller makes

For each arm of a running experiment, the controller creates a Deployment and
a Service named after the shop, the experiment, and the arm:

- The pods copy the shop's pod spec: its environment, Secrets, service
  account, resources, probes, and volumes. Only the image differs, and the
  `statespace-proxy` sidecar is left out.
- The pods carry only the controller's labels, so the shop's Service never
  sends them traffic. The proxies send them their subjects.
- Each arm gets the shop's replicas times its share of the subjects, at least
  one.
- The control arm runs the image the shop ran when the experiment started.

When an experiment stops, its arms are deleted. Deleting the controller's
Deployment deletes every arm it made.

Arms record every line their pods print. Kubernetes merges a container's
standard output and standard error, so every line is recorded as `stdout`.
