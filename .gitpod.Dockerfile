# Borrowed from https://github.com/gitpod-io/template-nix/blob/23e258d392bee430e713638de148e566a90e2b00/.gitpod.Dockerfile
# Borrowed from https://github.com/the-nix-way/nix-flakes-gitpod/blob/main/.gitpod.Dockerfile
FROM gitpod/workspace-base

USER root

# Install Nix
RUN addgroup --system nixbld \
	&& adduser gitpod nixbld \
	&& for i in $(seq 1 32); do useradd -ms /bin/bash nixbld$i &&  adduser nixbld$i nixbld; done \
	&& mkdir -m 0755 /nix && sudo chown gitpod /nix -R

RUN sudo chown gitpod /nix/store \
  	&& sudo chown gitpod /nix/var -R

# Install Flox
CMD /bin/bash -l
USER gitpod
ENV USER gitpod
WORKDIR /home/gitpod

RUN wget https://downloads.flox.dev/by-env/stable/deb/flox.x86_64-linux.deb \
	&& sudo dpkg -i ./flox.x86_64-linux.deb

