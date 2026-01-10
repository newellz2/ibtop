# ibtop

A CLI application for watching fabric throughput, congestion, and errors.

Use the Up and Down arrow keys to scroll through the node table when the list exceeds the available screen space. `Enter` will give you a details for a switch.

![image](https://github.com/user-attachments/assets/26ff51a4-d8c0-4b49-828d-b686f80fda39)

Can be built with the following commands:
```bash
git clone git@github.com:newellz2/ibmad.git
git clone git@github.com:newellz2/ibtop.git

cd ibtop

./build-docker.sh "3.1.0" "" "x86_64" "22.04"

./ibtop --help
```

## License

Copyright (c) Zach Newell <zlantern@gmail.com>

This project is licensed under the MIT license ([LICENSE] or <http://opensource.org/licenses/MIT>)

[LICENSE]: ./LICENSE
